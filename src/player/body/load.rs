//! Loading: the character's body (the record's mesh and skins, the pawn
//! class's animation names and bones) and the weapons' third-person
//! attachments (mesh, the animation names they give the pawn), each loaded
//! the first time a pawn holds that weapon.

use std::collections::HashMap;

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::Value;

use super::PawnState;
use crate::engine::runlog;
use crate::render::skinned::{SkinnedModel, Skins};
use crate::world::map::MapRequest;

/// KFPawn.GetWeaponBoneFor: the bone weapons attach to.
const WEAPON_BONE: &str = "WeaponR_Bone";
/// The player pawn class (its defaults and its parents': KFPawn, xPawn).
const PAWN_CLASS: &str = "KFMod.KFHumanPawn";

/// The package set, kept to load attachments later (it holds `Rc`s: a
/// non-send resource).
#[derive(Default)]
pub(super) struct BodyPackages(Option<PackageSet>);

/// The animation names a pawn plays: the pawn class's own, or the ones
/// KFPawn.SetWeaponAttachment copies from a KFWeaponAttachment.
#[derive(Clone, Debug, Default)]
pub(super) struct AnimNames {
    /// MovementAnims[0-3]: forward, back, left, right.
    pub movement: [String; 4],
    pub turn_left: String,
    pub turn_right: String,
    /// [Get4WayDirection]: forward, back, left, right.
    pub air: [String; 4],
    pub takeoff: [String; 4],
    pub land: [String; 4],
    pub air_still: String,
    pub takeoff_still: String,
    pub idle_weapon: String,
    pub fire: [String; 4],
    pub fire_alt: [String; 4],
    /// HitAnims: front, back, left, right.
    pub hit: [String; 4],
    pub post_fire_blend_stand: String,
}

impl AnimNames {
    /// Reads the names from a class's defaults, element by element up the
    /// class chain (UnrealScript array defaults inherit per element).
    fn read(defaults: &ClassDefaults, class: &ObjectHandle) -> Self {
        let name = |p: &str, i: u32| match defaults.get_at(class, p, i) {
            Some((Value::Name(n), pkg)) => pkg.pkg.name(n).to_string(),
            _ => String::new(),
        };
        let four = |p: &str| [0, 1, 2, 3].map(|i| name(p, i));
        AnimNames {
            movement: four("MovementAnims"),
            turn_left: name("TurnLeftAnim", 0),
            turn_right: name("TurnRightAnim", 0),
            air: four("AirAnims"),
            takeoff: four("TakeoffAnims"),
            land: four("LandAnims"),
            air_still: name("AirStillAnim", 0),
            takeoff_still: name("TakeoffStillAnim", 0),
            idle_weapon: name("IdleWeaponAnim", 0),
            fire: four("FireAnims"),
            fire_alt: four("FireAltAnims"),
            hit: four("HitAnims"),
            post_fire_blend_stand: name("PostFireBlendStandAnim", 0),
        }
    }

    fn all(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for set in [&self.movement, &self.air, &self.takeoff, &self.land, &self.fire, &self.fire_alt, &self.hit] {
            v.extend(set.iter().map(String::as_str));
        }
        v.extend([&self.turn_left, &self.turn_right, &self.air_still, &self.takeoff_still, &self.idle_weapon, &self.post_fire_blend_stand].map(String::as_str));
        v.retain(|n| !n.is_empty());
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// One character's body.
pub(super) struct BodyModel {
    pub name: String,
    pub model: SkinnedModel,
    /// KFPawn PrePivot and DrawScale.
    pub pre_pivot: Vec3,
    pub draw_scale: f32,
    /// The pawn class's own animation names (used without a KF attachment).
    pub names: AnimNames,
    /// FireRootBone (channel 1's root), SpineBone1 / SpineBone2 (the aim
    /// pitch), the weapon bone.
    pub fire_root: Option<usize>,
    pub spine: [Option<usize>; 2],
    pub weapon_bone: Option<usize>,
    /// The death ragdoll (rec.Ragdoll, from KarmaData/KF_Characters_Trip.ka,
    /// the zeds' file) and KFPawn's RagDeathVel / RagDeathUpKick.
    pub ragdoll: Option<crate::zeds::ragdoll::RagdollDef>,
    pub rag_death_vel: f32,
    pub rag_up_kick: f32,
}

/// A weapon's third-person attachment (its AttachmentClass).
pub(super) struct AttachmentDef {
    pub class: String,
    /// The mesh (None: drawn as nothing) and its DrawScale.
    pub model: Option<SkinnedModel>,
    pub draw_scale: f32,
    /// Bind-pose points in the attachment's mesh space.
    pub points: Vec<Vec3>,
    /// The names KFPawn.SetWeaponAttachment copies (None: not a
    /// KFWeaponAttachment, the pawn keeps its own).
    pub names: Option<AnimNames>,
    /// xWeaponAttachment bRapidFire / bAltRapidFire.
    pub rapid: [bool; 2],
    /// The weapon class's WeaponReloadAnim.
    pub reload_anim: String,
}

/// The loaded bodies and attachments.
#[derive(Resource, Default)]
pub(super) struct BodyModels {
    pub characters: Vec<BodyModel>,
    pub attachments: Vec<AttachmentDef>,
    /// Weapon class (lower case) -> attachment index (None: could not load).
    pub by_weapon: HashMap<String, Option<usize>>,
}

fn float(defaults: &ClassDefaults, class: &ObjectHandle, prop: &str, d: f32) -> f32 {
    match defaults.get(class, prop) {
        Some((Value::Float(f), _)) => f,
        Some((Value::Int(i), _)) => i as f32,
        _ => d,
    }
}

fn bool_prop(defaults: &ClassDefaults, class: &ObjectHandle, prop: &str) -> bool {
    matches!(defaults.get(class, prop), Some((Value::Bool(true), _)))
}

fn name_prop(defaults: &ClassDefaults, class: &ObjectHandle, prop: &str) -> String {
    match defaults.get(class, prop) {
        Some((Value::Name(n), pkg)) => pkg.pkg.name(n).to_string(),
        _ => String::new(),
    }
}

/// The `Skins` default of a class (as the zeds read it).
fn class_skins(defaults: &ClassDefaults, class: &ObjectHandle) -> Skins {
    match defaults.get(class, "Skins") {
        Some((Value::Array { count, raw }, p)) => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            Skins {
                refs: (0..count).filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw)).collect(),
                package: Some(p),
                named: Vec::new(),
            }
        }
        _ => Skins { refs: Vec::new(), package: None, named: Vec::new() },
    }
}

/// Loads the local player's character body (PostStartup).
pub(super) fn load_body_models(
    mut kept: NonSendMut<BodyPackages>,
    mut models: ResMut<BodyModels>,
    request: Res<MapRequest>,
    character: Res<crate::player::character::CharacterChoice>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let started = std::time::Instant::now();
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    match load_body(&set, &defaults, &request, character.0.as_deref(), &mut meshes, &mut images, &mut materials) {
        Ok(b) => {
            let names = &b.names;
            let mut wanted: Vec<&str> = names.all();
            wanted.extend(["Weapon_Switch", "DeathF", "DeathB", "DeathL", "DeathR"]);
            let missing: Vec<&str> = wanted.into_iter().filter(|n| b.model.sequence(n).is_none()).collect();
            let m = &b.model.mesh;
            runlog::kv(
                "body_loaded",
                &format!(
                    "character={} bones={} points={} triangles={} parts={} mesh_scale={:?} mesh_origin={:?} rot_origin={:?} pre_pivot={:?} draw_scale={} fire_root={:?} spine={:?} weapon_bone={:?} seconds={:.2} sequences={:?}",
                    b.name,
                    m.bones.len(),
                    m.points.len(),
                    m.triangles.len(),
                    b.model.parts.len(),
                    m.scale,
                    m.origin,
                    m.rot_origin,
                    b.pre_pivot.to_array(),
                    b.draw_scale,
                    b.fire_root.map(|i| m.bones[i].name.clone()),
                    b.spine.map(|s| s.map(|i| m.bones[i].name.clone())),
                    b.weapon_bone.map(|i| m.bones[i].name.clone()),
                    started.elapsed().as_secs_f64(),
                    b.model.anim.as_ref().map(|a| a.sequences.iter().map(|s| s.name.clone()).collect::<Vec<_>>())
                ),
            );
            if !missing.is_empty() {
                runlog::kv("body_anims_missing", &format!("character={} source=pawn_class anims={missing:?}", b.name));
            }
            models.characters.push(b);
        }
        Err(e) => runlog::kv("body_error", &format!("error=\"{e}\"")),
    }
    kept.0 = Some(set);
}

#[allow(clippy::too_many_arguments)]
fn load_body(
    set: &PackageSet,
    defaults: &ClassDefaults,
    request: &MapRequest,
    wanted: Option<&str>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<BodyModel, String> {
    let (record, _species, _why) =
        crate::player::character::pick(set, defaults, &request.install_root, wanted).ok_or("no character record")?;
    let mesh_h = set.find_object(&record.mesh, None).ok_or(format!("mesh {} not found", record.mesh))?;
    // SetTeamSkin: Skins[0] = BodySkin, Skins[1] = FaceSkin.
    let find = |path: &str| if path.is_empty() { None } else { set.find_object(path, None) };
    let skins = Skins { refs: Vec::new(), package: None, named: vec![find(&record.body_skin), find(&record.face_skin)] };
    runlog::kv(
        "body_record",
        &format!(
            "character={} mesh={} body_skin={} ({}) face_skin={} ({})",
            record.name,
            record.mesh,
            record.body_skin,
            if skins.named[0].is_some() { "found" } else { "missing" },
            record.face_skin,
            if skins.named[1].is_some() { "found" } else { "missing" },
        ),
    );
    let model = SkinnedModel::load(set, &mesh_h, &skins, true, meshes, images, materials)?;
    let pawn = set.find_object(PAWN_CLASS, Some("Class")).ok_or("pawn class not found")?;
    let pre_pivot = match defaults.get(&pawn, "PrePivot") {
        Some((Value::Vector(v), _)) => Vec3::from_array(v),
        _ => Vec3::ZERO,
    };
    let bone = |p: &str| model.find_bone(&name_prop(defaults, &pawn, p));
    // xPawn.PlayDyingAnimation: RagdollOverride (rec.Ragdoll) names the
    // Karma skeleton; KF keeps its human ragdolls with the zeds'.
    let ka_path = request.install_root.join("KarmaData").join("KF_Characters_Trip.ka");
    let ragdoll = match std::fs::read(&ka_path)
        .map_err(|e| e.to_string())
        .and_then(|b| ue_assets::karma::parse_ka(&String::from_utf8_lossy(&b)))
    {
        Ok(all) => match all.get(&record.ragdoll) {
            Some(ka) => match crate::zeds::ragdoll::RagdollDef::new(ka, &model) {
                Ok((def, report)) => {
                    runlog::kv("body_ragdoll_loaded", &format!("character={} {report}", record.name));
                    Some(def)
                }
                Err(e) => {
                    runlog::kv("body_ragdoll_error", &format!("character={} ragdoll={} error=\"{e}\"", record.name, record.ragdoll));
                    None
                }
            },
            None => {
                runlog::kv("body_ragdoll_error", &format!("character={} ragdoll=\"{}\" error=\"not in file\"", record.name, record.ragdoll));
                None
            }
        },
        Err(e) => {
            runlog::kv("body_ragdoll_error", &format!("path={} error=\"{e}\"", ka_path.display()));
            None
        }
    };
    Ok(BodyModel {
        ragdoll,
        rag_death_vel: float(defaults, &pawn, "RagDeathVel", 0.0),
        rag_up_kick: float(defaults, &pawn, "RagDeathUpKick", 0.0),
        name: record.name.clone(),
        pre_pivot,
        draw_scale: float(defaults, &pawn, "DrawScale", 1.0),
        names: AnimNames::read(defaults, &pawn),
        fire_root: bone("FireRootBone"),
        spine: [bone("SpineBone1"), bone("SpineBone2")],
        weapon_bone: model.find_bone(WEAPON_BONE),
        model,
    })
}

/// Loads the attachment of every weapon a pawn holds, the first time.
#[allow(clippy::too_many_arguments)]
pub(super) fn load_attachments(
    kept: NonSend<BodyPackages>,
    mut models: ResMut<BodyModels>,
    pawns: Query<&PawnState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(set) = kept.0.as_ref() else { return };
    for s in &pawns {
        let Some(class) = s.weapon_class.as_ref() else { continue };
        let key = class.to_ascii_lowercase();
        if models.by_weapon.contains_key(&key) {
            continue;
        }
        let defaults = ClassDefaults::new(set);
        let index = match load_attachment(set, &defaults, class, &mut meshes, &mut images, &mut materials) {
            Ok(a) => {
                let missing: Vec<String> = match (&a.names, models.characters.first()) {
                    (Some(n), Some(b)) => {
                        let mut v = n.all();
                        v.push(&a.reload_anim);
                        v.into_iter().filter(|x| !x.is_empty() && b.model.sequence(x).is_none()).map(str::to_string).collect()
                    }
                    _ => Vec::new(),
                };
                runlog::kv(
                    "body_attachment",
                    &format!(
                        "weapon={class} attachment={} mesh={} draw_scale={} kf_anims={} rapid={:?} reload_anim={} movement={:?} idle={} fire={} fire_alt={} hit={:?}",
                        a.class,
                        a.model.as_ref().map_or("none".to_string(), |m| format!("{}x{}tris", m.mesh.points.len(), m.mesh.triangles.len())),
                        a.draw_scale,
                        a.names.is_some(),
                        a.rapid,
                        a.reload_anim,
                        a.names.as_ref().map(|n| n.movement.clone()),
                        a.names.as_ref().map_or("", |n| n.idle_weapon.as_str()),
                        a.names.as_ref().map_or("", |n| n.fire[0].as_str()),
                        a.names.as_ref().map_or("", |n| n.fire_alt[0].as_str()),
                        a.names.as_ref().map(|n| n.hit.clone()),
                    ),
                );
                if !missing.is_empty() {
                    runlog::kv("body_anims_missing", &format!("source={} anims={missing:?}", a.class));
                }
                models.attachments.push(a);
                Some(models.attachments.len() - 1)
            }
            Err(e) => {
                runlog::kv("body_attachment_error", &format!("weapon={class} error=\"{e}\""));
                None
            }
        };
        models.by_weapon.insert(key, index);
    }
}

fn load_attachment(
    set: &PackageSet,
    defaults: &ClassDefaults,
    weapon_class: &str,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<AttachmentDef, String> {
    let weapon = set.find_object(weapon_class, Some("Class")).ok_or("weapon class not found")?;
    let class = match defaults.get(&weapon, "AttachmentClass") {
        Some((Value::Object(r), pkg)) => set.resolve(&pkg, r).ok_or("AttachmentClass not found")?,
        _ => return Err("no AttachmentClass".into()),
    };
    // The mesh: Mesh, else MeshRef (KFWeaponAttachment.PreloadAssets).
    let mesh_h = match defaults.get(&class, "Mesh") {
        Some((Value::Object(r), pkg)) => set.resolve(&pkg, r),
        _ => None,
    }
    .or_else(|| match defaults.get(&class, "MeshRef") {
        Some((Value::Str(path), _)) => set.find_object(&path, None),
        _ => None,
    });
    let draw_scale = float(defaults, &class, "DrawScale", 1.0);
    let model = match &mesh_h {
        Some(h) => Some(SkinnedModel::load(set, h, &class_skins(defaults, &class), true, meshes, images, materials)?),
        None => None,
    };
    let points = model.as_ref().map_or(Vec::new(), |m| m.pose_with_bones(None, 0.0).0);
    Ok(AttachmentDef {
        class: class.path(),
        names: defaults.is_a(&class, "KFWeaponAttachment").then(|| AnimNames::read(defaults, &class)),
        rapid: [bool_prop(defaults, &class, "bRapidFire"), bool_prop(defaults, &class, "bAltRapidFire")],
        reload_anim: name_prop(defaults, &weapon, "WeaponReloadAnim"),
        draw_scale,
        points,
        model,
    })
}

type RenderAssets<'w> = (ResMut<'w, Assets<Mesh>>, ResMut<'w, Assets<Image>>, ResMut<'w, Assets<StandardMaterial>>);

/// A character change after startup (the perk page's Change Character,
/// then SAVE: `CharacterChoice` is set by the weapon code's sleeve swap):
/// the body model is loaded again and every body is despawned, so
/// `spawn_bodies` gives the pawns the new one. KF swaps the character with
/// the next pawn (the lobby's Ready spawns it); ours already exists.
#[allow(clippy::too_many_arguments)]
pub(super) fn reload_on_character_change(
    mut commands: Commands,
    kept: NonSend<BodyPackages>,
    mut models: ResMut<BodyModels>,
    request: Res<MapRequest>,
    character: Res<crate::player::character::CharacterChoice>,
    bodies: Query<(Entity, &super::PawnBody)>,
    (mut meshes, mut images, mut materials): RenderAssets,
) {
    if !character.is_changed() || character.is_added() {
        return;
    }
    let Some(set) = kept.0.as_ref() else { return };
    let wanted = character.0.as_deref();
    if models.characters.first().is_some_and(|b| wanted.is_some_and(|w| b.name.eq_ignore_ascii_case(w))) {
        return;
    }
    let defaults = ClassDefaults::new(set);
    match load_body(set, &defaults, &request, wanted, &mut meshes, &mut images, &mut materials) {
        Ok(b) => {
            runlog::kv("body_reloaded", &format!("character={} bodies_respawned={}", b.name, bodies.iter().count()));
            models.characters.clear();
            models.characters.push(b);
            for (e, body) in &bodies {
                commands.entity(body.root).despawn();
                commands.entity(e).remove::<super::PawnBody>();
            }
        }
        Err(e) => runlog::kv("body_error", &format!("error=\"{e}\" reason=character_change")),
    }
}
