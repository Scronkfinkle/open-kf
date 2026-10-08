//! KF particle effects (UE2 Emitter actors), simulated and drawn here. The
//! effect definitions come from the game (`ue_assets::emitter`); the
//! simulation rebuilds what the engine does natively, from what each
//! setting means. Rules not visible in the data are marked "assumed".
//!
//! Simulation runs in Unreal units and axes; positions are converted to
//! Bevy when the meshes are built. Each sub-emitter is drawn as one mesh
//! rebuilt every frame (one per material section for mesh emitters).

use std::collections::HashMap;
use std::sync::Arc;

use avian3d::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;

use ue_assets::class_defaults::ClassDefaults;
use ue_assets::emitter::{EmitterDef, EmitterKind, read_emitter_class};
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::Rotator;
use ue_assets::static_mesh::read_static_mesh;
use ue_assets::texture::{decode_rgba, read_texture};

use crate::engine::camera::FlyCamera;
use crate::engine::coords;
use crate::world::map::MapRequest;
use crate::engine::runlog;

/// Effects loaded at startup (the gore effects, see DESIGN.md).
const EFFECT_CLASSES: [&str; 88] = [
    // KFGlassMover GlassBits / BreakGlassBits (glass.rs).
    "KFMod.WindowGlassEmitter",
    "KFMod.BreakWindowGlassEmitter",
    // KFDoorMover Wood / MetalDoorExplodeEffectClass (door.rs GoBang).
    "KFMod.KFDoorExplodeWood",
    "KFMod.KFDoorExplodeMetal",
    // KFWelderHitEffect.HitEffectClasses (door.rs).
    "KFMod.WelderHitEmitter",
    // ZEDGunAltFire.ChargeEmitterClass.
    "ROEffects.ChargeUp1stZEDGun",
    // ZED guns: bolt trails and impacts, the MKII's zap orb.
    "KFMod.ZEDProjectileTrail",
    "KFMod.ZEDProjectileImpact",
    "KFMod.ZEDMKIIPrimaryProjectileTrail",
    "KFMod.ZEDMKIIPrimaryProjectileImpact",
    "KFMod.ZEDMKIISecondaryProjectileTrail",
    "KFMod.ZEDMKIISecondaryProjectileExplosion",
    // Husk Gun: the charge glow (HuskGunFire.ChargeEmitterClass), fireball
    // trails and explosions by charge (HuskGunProjectile _Weak / _Strong).
    "ROEffects.ChargeUp1stHusk",
    "KFMod.FlameThrowerHusk_Weak",
    "KFMod.FlameThrowerHusk_Medium",
    "KFMod.FlameThrowerHusk_Strong",
    "KFMod.FlameImpact_Weak",
    "KFMod.FlameImpact_Medium",
    "KFMod.FlameImpact_Strong",
    // Flamethrower flames (FlameTendril's trail, Explode's FuelFlame).
    "KFMod.FlameThrowerFlameB",
    "KFMod.FuelFlame",
    // Burning zeds (KFMonster.BurnEffect).
    "KFMod.KFMonsterFlame",
    // Frag explosions (Nade.Explode).
    "KFMod.KFNadeExplosion",
    // The Firebug's FlameNade (FlameNade.Explode).
    "KFMod.KFIncendiaryExplosion",
    // The Medic's grenade (MedicNade.Explode).
    "KFMod.KFNadeHealing",
    // Grenade explosions (M79GrenadeProjectile.Explode).
    "KFMod.KFNadeLExplosion",
    "KFMod.DismembermentJetHead",
    "KFMod.DismembermentJetDecapitate",
    "KFMod.DismembermentJetLimb",
    "ROEffects.BrainSplash",
    "ROEffects.ROBloodSpurt",
    "ROEffects.BloodTrail",
    "KFMod.KFGibJet",
    "ROEffects.ROBloodPuff",
    "ROEffects.ROBloodPuffSmall",
    "ROEffects.ROBloodPuffMedium",
    "ROEffects.ROBloodPuffLarge",
    "ROEffects.KFVomitJet",
    "KFMod.BileExplosion",
    "KFMod.BileExplosionHeadless",
    "ROEffects.SirenScream",
    // A projectile the scream destroys (Nade / LAWProj / M79GrenadeProjectile
    // / PipeBombProjectile.Disintegrate).
    "KFMod.SirenNadeDeflect",
    "ROEffects.HuskChargeUp",
    "ROEffects.HuskMuzzle",
    "KFMod.FlameImpact",
    "KFMod.FlameThrowerFlameB",
    "ROEffects.PanzerfaustTrail",
    "KFMod.LawExplosion",
    "ROEffects.MuzzleFlash1stMP",
    "ROEffects.KFShellEject9mm",
    "KFMod.KFNewTracer",
    "ROEffects.MuzzleFlash3rdMG",
    "ROEffects.ROBulletHitRockEffect",
    // Every FlashEmitterClass / ShellEjectClass of the base weapons.
    "KFMod.KFShellEjectFAL",
    "KFMod.KSGShellEject",
    "KFMod.MK23Shell",
    "KFMod.MuzzleFlashMK",
    "KFMod.ShellEjectKriss",
    "KFMod.TrenchgunMuzzFlash",
    "KFMod.ZEDMKIIPrimaryMuzzleFlash1P",
    "ROEffects.KFShellEjectAK",
    "ROEffects.KFShellEjectBenelli",
    "ROEffects.KFShellEjectBullpup",
    "ROEffects.KFShellEjectEBR",
    "ROEffects.KFShellEjectHandCannon",
    "ROEffects.KFShellEjectM4Rifle",
    "ROEffects.KFShellEjectMP",
    "ROEffects.KFShellEjectMP5SMG",
    "ROEffects.KFShellEjectMac",
    "ROEffects.KFShellEjectMkb",
    "ROEffects.KFShellEjectSCAR",
    "ROEffects.KFShellEjectShotty",
    "ROEffects.MuzzleFlash1stHusk",
    "ROEffects.MuzzleFlash1stKar",
    "ROEffects.MuzzleFlash1stNadeL",
    "ROEffects.MuzzleFlash1stNailGun",
    "ROEffects.MuzzleFlash1stPTRD",
    "ROEffects.MuzzleFlash1stSTG",
    "ROEffects.MuzzleFlash1stZEDGunPrimary",
    "ROEffects.ZEDGunChargeDown",
    // Other players' weapons in a network game: every base weapon
    // attachment's mMuzFlashClass (KFWeaponAttachment.DoFlashEmitter).
    "ROEffects.MuzzleFlash3rdPistol",
    "ROEffects.MuzzleFlash3rdMP",
    "ROEffects.MuzzleFlash3rdKar",
    "ROEffects.MuzzleFlash3rdNadeL",
    "ROEffects.MuzzleFlash3rdPTRD",
    "ROEffects.MuzzleFlash3rdNailGun",
    "ROEffects.MuzzleFlash3rdFlareRevolver",
    "KFMod.KFLawMuzzFlash",
];

pub struct ParticlePlugin;

impl Plugin for ParticlePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ModulateMaterial>::default())
            .add_systems(PostStartup, load_library)
            .add_systems(Update, spawn_map_emitters)
            .add_systems(PostUpdate, update_effects);
        app.world_mut()
            .resource_mut::<Assets<bevy::shader::Shader>>()
            .insert(&MODULATE_SHADER, bevy::shader::Shader::from_wgsl(MODULATE_WGSL, "particles.rs/modulate.wgsl"))
            .expect("shader handle");
    }
}

/// UE2 Modulated particles: the scene behind is multiplied by the texture
/// (already doubled at load, so white = no change), faded toward "no
/// change" by the vertex alpha. A shader of its own because the standard
/// material tone-maps its output (the camera is not HDR), which turns
/// white into grey and darkened everything behind the particles.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct ModulateMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
}

const MODULATE_SHADER: Handle<bevy::shader::Shader> = bevy::asset::uuid_handle!("6f1d2a8e-3b4c-4d5e-9f60-718293a4b5c6");

/// Bevy's Multiply blend computes dst * src + (1 - src.a) * dst, so the
/// colour is premultiplied by the alpha here.
const MODULATE_WGSL: &str = r#"
#import bevy_pbr::forward_io::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var particle_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var particle_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(particle_texture, particle_sampler, in.uv);
    let a = in.color.a;
    return vec4<f32>(t.rgb * in.color.rgb * a, a);
}
"#;

impl Material for ModulateMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        MODULATE_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Multiply
    }

    /// No back-face culling: sprites are seen from either side (the
    /// default culling hid every camera-facing sprite).
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// A sprite emitter's material: modulated ones use their own shader.
#[derive(Clone)]
enum SpriteMaterial {
    Standard(Handle<StandardMaterial>),
    Modulate(Handle<ModulateMaterial>),
}

/// A static mesh kept on the CPU, to be copied once per particle.
struct MeshSection {
    /// Unreal mesh space.
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    material: Handle<StandardMaterial>,
}

struct LoadedEmitter {
    def: EmitterDef,
    /// Sprites: the material for the texture and draw style.
    material: Option<SpriteMaterial>,
    /// Mesh emitters: the mesh's sections.
    mesh: Vec<MeshSection>,
}

pub struct LoadedEffect {
    class: String,
    emitters: Vec<LoadedEmitter>,
    life_span: f32,
}

/// The loaded effects, by class path (e.g. "KFMod.DismembermentJetHead").
#[derive(Resource, Default)]
pub struct EffectLibrary(HashMap<String, Arc<LoadedEffect>>);

/// A running effect. `frame` is its location and axes in Unreal world space;
/// whoever it is attached to updates it.
#[derive(Component)]
pub struct ParticleEffect {
    effect: Arc<LoadedEffect>,
    /// Kept when it has no particles (a muzzle flash or shell ejector that
    /// the weapon spawns once and triggers per shot); removed by its owner.
    persistent: bool,
    pub frame: (Vec3, Mat3),
    id: u32,
    age: f32,
    emitters: Vec<EmitterState>,
    rng: u32,
    log_timer: f32,
    /// Actor LifeSpan (the class default unless the spawner set one).
    life_span: f32,
    /// Emitter.Kill(): no new particles; removed when the last one dies.
    killed: bool,
}

/// Particles each emitter spawns on Trigger, for classes whose script
/// overrides Trigger with SpawnParticle calls (every KF muzzle flash and
/// shell ejector does; the engine's own Trigger, which toggles emitters
/// with TriggerDisabled, is never what these use).
const TRIGGER_COUNTS: [(&str, &[u32]); 32] = [
    // MuzzleFlash1stMP.Trigger: Emitters[0] 2, Emitters[1] 1.
    ("ROEffects.MuzzleFlash1stMP", &[2, 1]),
    // KFShellEject9mm.Trigger: the casing, and 3 smoke puffs.
    ("ROEffects.KFShellEject9mm", &[1, 3]),
    ("ROEffects.MuzzleFlash3rdMG", &[2, 2, 2, 2, 2, 1, 3, 3]),
    ("ROEffects.MuzzleFlash3rdPistol", &[2, 2, 2, 2, 2, 1, 2, 2]),
    // KFShellEject / ROMuzzleFlash1st / ROMuzzleFlash3rd: Emitters[0] 1.
    ("ROEffects.KFShellEject", &[1]),
    // The other base weapons' flashes and shell ejectors: each class's
    // Trigger (TrenchgunMuzzFlash inherits MuzzleFlash1stKar's).
    ("KFMod.KFShellEjectFAL", &[1, 1]),
    ("KFMod.KSGShellEject", &[1, 3]),
    ("KFMod.MK23Shell", &[1, 3]),
    ("KFMod.MuzzleFlashMK", &[2, 1]),
    ("KFMod.ShellEjectKriss", &[1, 3]),
    ("KFMod.TrenchgunMuzzFlash", &[2, 1]),
    ("KFMod.ZEDMKIIPrimaryMuzzleFlash1P", &[2, 1, 5, 5]),
    ("ROEffects.KFShellEjectAK", &[1, 1]),
    ("ROEffects.KFShellEjectBenelli", &[1, 3]),
    ("ROEffects.KFShellEjectBullpup", &[1, 1]),
    ("ROEffects.KFShellEjectEBR", &[1, 1]),
    ("ROEffects.KFShellEjectHandCannon", &[1, 3]),
    ("ROEffects.KFShellEjectM4Rifle", &[1, 1]),
    ("ROEffects.KFShellEjectMP", &[1, 3]),
    ("ROEffects.KFShellEjectMP5SMG", &[1, 3]),
    ("ROEffects.KFShellEjectMac", &[1, 1]),
    ("ROEffects.KFShellEjectMkb", &[1, 1]),
    ("ROEffects.KFShellEjectSCAR", &[1, 1]),
    ("ROEffects.KFShellEjectShotty", &[1, 3]),
    ("ROEffects.MuzzleFlash1stHusk", &[3, 3]),
    ("ROEffects.MuzzleFlash1stKar", &[2, 1]),
    ("ROEffects.MuzzleFlash1stNadeL", &[3, 1]),
    ("ROEffects.MuzzleFlash1stNailGun", &[2]),
    ("ROEffects.MuzzleFlash1stPTRD", &[2, 1]),
    ("ROEffects.MuzzleFlash1stSTG", &[2, 1]),
    ("ROEffects.MuzzleFlash1stZEDGunPrimary", &[2, 1, 5, 5]),
    ("ROEffects.ZEDGunChargeDown", &[3, 3]),
];

impl ParticleEffect {
    /// The effect's script Trigger (TRIGGER_COUNTS).
    pub fn trigger(&mut self) {
        let class = self.effect.class.as_str();
        match TRIGGER_COUNTS.iter().find(|(c, _)| c.eq_ignore_ascii_case(class)) {
            Some((_, counts)) => {
                for (i, n) in counts.iter().enumerate() {
                    self.spawn_particles(i, *n);
                }
            }
            None => runlog::kv("effect_trigger_unknown", &format!("class={class}")),
        }
    }

    /// ParticleEmitter.SpawnParticle(n) on one emitter: spawned on the next
    /// update, at most MaxParticles alive (the oldest make room; assumed
    /// from UE2's fixed particle array).
    pub fn spawn_particles(&mut self, emitter: usize, n: u32) {
        if let Some(s) = self.emitters.get_mut(emitter) {
            s.pending += n;
        }
    }

    /// Emitter.SpawnParticle(n): n on every emitter.
    pub fn spawn_all(&mut self, n: u32) {
        for i in 0..self.emitters.len() {
            self.spawn_particles(i, n);
        }
    }

    /// What a script does before SpawnParticle on a tracer: set one
    /// emitter's StartVelocityRange to a single value (Unreal units/s, in
    /// the effect's axes) and its LifetimeRange to a single time.
    pub fn set_start(&mut self, emitter: usize, velocity: Vec3, lifetime: f32) {
        if let Some(s) = self.emitters.get_mut(emitter) {
            s.start_velocity = Some(velocity);
            s.lifetime = Some(lifetime);
        }
    }

    /// Emitter.Kill(): stop spawning and go away once the particles are gone.
    pub fn kill(&mut self) {
        if !self.killed {
            self.killed = true;
            runlog::kv("effect_killed", &format!("effect={} class={} age={:.2}", self.id, self.effect.class, self.age));
        }
    }
}

/// One sub-emitter's particles, kept as KF keeps them: a ring of at most
/// MaxParticles slots. A new particle goes into slot `next` (replacing
/// whatever was there, alive or not); `slots.len()` is how many slots have
/// ever been used (KF's ActiveParticles). Dead particles stay in their
/// slot (not drawn) until it is reused.
struct EmitterState {
    slots: Vec<Particle>,
    next: usize,
    /// Asked for by SpawnParticle / Trigger, spawned on the next update.
    pending: u32,
    /// StartVelocityRange / LifetimeRange as last set by a script.
    start_velocity: Option<Vec3>,
    lifetime: Option<f32>,
    /// Particles spawned so far, and the fractional spawn carry-over.
    spawned: u32,
    carry: f32,
    /// Spawn rate used on the last update (particles per second), for the log.
    rate: f32,
    /// Seconds left before the emitter starts (InitialDelayRange).
    delay: f32,
    /// One mesh per drawn section.
    meshes: Vec<Handle<Mesh>>,
}

#[derive(Clone, Copy)]
struct Particle {
    /// False once its lifetime is over (until its slot is reused).
    alive: bool,
    /// Effect-local for CoordinateSystem Relative, else world (Unreal units).
    pos: Vec3,
    vel: Vec3,
    size: Vec3,
    age: f32,
    life: f32,
    /// Spin (revolutions) and its rate (revolutions per second), X/Y/Z.
    spin: Vec3,
    spin_rate: Vec3,
    damping: Vec3,
    velocity_loss: Vec3,
    subdivision: u32,
}

fn frand(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    (*rng % 100_000) as f32 / 100_000.0
}

fn in_range(rng: &mut u32, (lo, hi): (f32, f32)) -> f32 {
    lo + (hi - lo) * frand(rng)
}

fn in_ranges(rng: &mut u32, r: &[(f32, f32); 3]) -> Vec3 {
    Vec3::new(in_range(rng, r[0]), in_range(rng, r[1]), in_range(rng, r[2]))
}

/// Unreal direction <-> Bevy direction.
fn to_bevy_dir(v: Vec3) -> Vec3 {
    coords::dir(v.to_array())
}

fn to_ue_dir(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, v.y)
}

/// Decodes a texture's top mip; `opaque` sets alpha to 255 (modulated and
/// additive drawing use the colour only). `modulate2x` doubles the colour:
/// UE2's Modulated particles multiply by 2 x texture (mid-grey = no change;
/// KF's blood textures have a grey background). Multiply blending cannot
/// brighten, so anything above mid-grey is clamped to "no change".
pub fn decode(h: &ObjectHandle, opaque: bool, modulate2x: bool, images: &mut Assets<Image>) -> Option<Handle<Image>> {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let tex = read_texture(&h.package.pkg, h.export).ok()?;
    let mip = tex.mips.first()?;
    let mut rgba = decode_rgba(tex.format, mip, None)?;
    for p in rgba.as_chunks_mut::<4>().0.iter_mut() {
        if opaque {
            p[3] = 255;
        }
        if modulate2x {
            for c in &mut p[..3] {
                *c = c.saturating_mul(2);
            }
        }
    }
    let image = Image::new(
        Extent3d {
            width: mip.width as u32,
            height: mip.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    Some(images.add(image))
}

/// The Bevy blend for an EParticleDrawStyle (Modulated has its own material).
fn alpha_mode(draw_style: u8) -> AlphaMode {
    match draw_style {
        1 | 4 => AlphaMode::Blend,
        // Darken: multiply what is behind.
        2 | 5 => AlphaMode::Multiply,
        // Translucent and Brighten: add.
        3 | 6 => AlphaMode::Add,
        _ => AlphaMode::Mask(0.5),
    }
}

fn load_library(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut modulate: ResMut<Assets<ModulateMaterial>>,
) {
    let started = std::time::Instant::now();
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    let mut library = EffectLibrary::default();
    for class in EFFECT_CLASSES {
        let effect = match read_emitter_class(&set, &defaults, class) {
            Ok(e) => e,
            Err(e) => {
                runlog::kv("effect_error", &format!("class={class} error=\"{e}\""));
                continue;
            }
        };
        let loaded = load_effect(effect, class, &set, &mut images, &mut materials, &mut modulate);
        library.0.insert(class.to_string(), Arc::new(loaded));
    }
    // Emitters placed in the map (fires, smoke...): loaded under
    // "map:<name>" and spawned once by `spawn_map_emitters`.
    let mut placed = MapEmitters::default();
    let path = request.install_root.join("Maps").join(format!("{}.rom", request.map));
    if let Ok(map) = set.load_path(&path) {
        let (mut ok, mut failed) = (0usize, Vec::new());
        for i in 0..map.pkg.exports.len() {
            let Some(class) = defaults.class_of(&map, i) else { continue };
            if !defaults.is_a(&class, "Emitter") {
                continue;
            }
            let name = map.pkg.object_name(ObjectRef::Export(i)).to_string();
            let effect = match ue_assets::emitter::read_emitter_actor(&set, &defaults, &map, i) {
                Ok(e) => e,
                Err(e) => {
                    failed.push(format!("{name}:{e}"));
                    continue;
                }
            };
            let props = ue_assets::properties::read_export_properties(&map.pkg, i).ok();
            let get = |n: &str| props.as_ref().and_then(|p| p.get(&map.pkg, n).cloned());
            let location = match get("Location") {
                Some(ue_assets::properties::Value::Vector(v)) => Vec3::from_array(v),
                _ => continue,
            };
            let rotation = match get("Rotation") {
                Some(ue_assets::properties::Value::Rotator(r)) => r,
                _ => Rotator::default(),
            };
            let key = format!("map:{name}");
            let loaded = load_effect(effect, &key, &set, &mut images, &mut materials, &mut modulate);
            library.0.insert(key.clone(), Arc::new(loaded));
            placed.0.push((key, location, rotation));
            ok += 1;
        }
        runlog::kv("map_emitters", &format!("loaded={ok} failed={} [{}]", failed.len(), failed.iter().take(8).cloned().collect::<Vec<_>>().join(" | ")));
    }
    commands.insert_resource(placed);
    runlog::kv("effects_ready", &format!("count={} seconds={:.2}", library.0.len(), started.elapsed().as_secs_f64()));
    commands.insert_resource(library);
}

/// Builds a loaded effect (materials, meshes) from an emitter definition.
fn load_effect(
    effect: ue_assets::emitter::EmitterEffect,
    class: &str,
    set: &PackageSet,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    modulate: &mut Assets<ModulateMaterial>,
) -> LoadedEffect {
    let mut emitters = Vec::new();
    let mut notes = Vec::new();
    for (def, assets) in effect.emitters {
        let mut loaded = LoadedEmitter {
            def,
            material: None,
            mesh: Vec::new(),
        };
        let d = &loaded.def;
        match d.kind {
            EmitterKind::Sprite => {
                let opaque = matches!(d.draw_style, 2 | 3 | 5 | 6);
                let mut chain = String::new();
                let image = assets.texture.as_ref().and_then(|t| {
                    let m = ue_assets::material::resolve(set, t, ObjectRef::Export(t.export));
                    chain = format!("{}:{}", t.package.pkg.object_name(ObjectRef::Export(t.export)), m.chain.join(">"));
                    m.texture.as_ref().and_then(|tex| decode(tex, opaque, d.draw_style == 2, images))
                });
                notes.push(format!("{}:sprite:texture={}:style={}:{chain}", d.name, image.is_some(), d.draw_style));
                // No texture (Texture = None, e.g. KF-WestLondon Emitter23,
                // WelderHitEmitter's SpriteEmitter42): not drawn. Assumed
                // from KF: drawn untextured they are grey squares.
                loaded.material = image.map(|image| match d.draw_style {
                    2 => SpriteMaterial::Modulate(modulate.add(ModulateMaterial { texture: image })),
                    _ => SpriteMaterial::Standard(materials.add(StandardMaterial {
                        base_color_texture: Some(image),
                        unlit: true,
                        alpha_mode: alpha_mode(d.draw_style),
                        cull_mode: None,
                        double_sided: true,
                        ..default()
                    })),
                });
            }
            EmitterKind::Mesh => {
                loaded.mesh = assets
                    .static_mesh
                    .as_ref()
                    .map(|h| load_mesh(set, h, images, materials))
                    .unwrap_or_default();
                notes.push(format!("{}:mesh:sections={}", d.name, loaded.mesh.len()));
            }
            _ => notes.push(format!("{}:{:?}:not_drawn", d.name, d.kind)),
        }
        emitters.push(loaded);
    }
    runlog::kv("effect_loaded", &format!("class={class} life_span={} emitters=[{}]", effect.life_span, notes.join(" ")));
    LoadedEffect {
        class: class.to_string(),
        emitters,
        life_span: effect.life_span,
    }
}

/// Emitters placed in the map: (library key, Unreal location, rotation).
#[derive(Resource, Default)]
pub struct MapEmitters(pub Vec<(String, Vec3, Rotator)>);

/// Starts the map's placed emitters once (on the sky layer when they are
/// in the sky zone).
fn spawn_map_emitters(
    mut commands: Commands,
    library: Option<Res<EffectLibrary>>,
    placed: Option<Res<MapEmitters>>,
    zones: Option<Res<crate::world::zones::Zones>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut done: Local<bool>,
) {
    let (Some(library), Some(placed)) = (library, placed) else { return };
    if *done {
        return;
    }
    *done = true;
    let mut sky = 0;
    for (i, (key, location, rotation)) in placed.0.iter().enumerate() {
        let in_sky = zones.as_deref().is_some_and(|z| {
            z.zones.get(z.bsp.point_zone(location.to_array())).is_some_and(|zf| zf.name.contains("SkyZone"))
        });
        if in_sky {
            sky += 1;
        }
        let options = SpawnOptions {
            layer: in_sky.then_some(crate::world::map::SKY_LAYER),
            ..default()
        };
        spawn_effect_with(&mut commands, &library, &mut meshes, key, *location, coords::ue_rotation_matrix(*rotation), 7000 + i as u32, options);
    }
    runlog::kv("map_emitters_spawned", &format!("count={} in_sky={sky}", placed.0.len()));
}

fn load_mesh(
    set: &PackageSet,
    h: &ObjectHandle,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Vec<MeshSection> {
    let Ok(sm) = read_static_mesh(&h.package.pkg, h.export) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (si, section) in sm.sections.iter().enumerate() {
        let tris = &sm.indices[section.first_index..section.first_index + section.num_triangles * 3];
        if tris.is_empty() {
            continue;
        }
        let rf = sm.materials.get(si).copied().unwrap_or(ObjectRef::Null);
        let simple = ue_assets::material::resolve(set, h, rf);
        let image = simple.texture.as_ref().and_then(|t| crate::render::skinned::decode_image(t, images));
        // The section material's blending (e.g. the Siren's scream ball is
        // additive); see-through sections are unlit, as KF's effects are.
        use ue_assets::material::Blend;
        let alpha_mode = match simple.blend {
            Blend::Additive => AlphaMode::Add,
            Blend::Translucent => AlphaMode::Blend,
            Blend::Masked => AlphaMode::Mask(0.5),
            _ => AlphaMode::Opaque,
        };
        runlog::kv(
            "effect_mesh_material",
            &format!(
                "mesh={} section={si} texture={:?} blend={:?}",
                h.package.pkg.object_name(ObjectRef::Export(h.export)),
                simple.texture.as_ref().map(|t| t.package.pkg.object_name(ObjectRef::Export(t.export)).to_string()),
                simple.blend
            ),
        );
        out.push(MeshSection {
            positions: sm.positions.iter().map(|p| Vec3::from_array(*p)).collect(),
            normals: sm.normals.iter().map(|n| Vec3::from_array(*n)).collect(),
            uvs: sm.uvs.first().cloned().unwrap_or_else(|| vec![[0.0, 0.0]; sm.positions.len()]),
            indices: tris.iter().map(|&i| i as u32).collect(),
            material: materials.add(StandardMaterial {
                base_color_texture: image,
                perceptual_roughness: 0.6,
                unlit: !matches!(alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_)),
                alpha_mode,
                cull_mode: None,
                double_sided: true,
                ..default()
            }),
        });
    }
    out
}

/// A particle mesh with every attribute it will ever get (the GPU mesh
/// allocator must not see the layout change) and one invisible triangle
/// (it must not see an empty mesh either: both caused "use-after-free"
/// errors from bevy_render's slab allocator).
fn empty_mesh(meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    write_mesh(&mut mesh, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    meshes.add(mesh)
}

/// Replaces a particle mesh's contents; an empty list becomes one
/// zero-size triangle.
fn write_mesh(
    mesh: &mut Mesh,
    mut positions: Vec<[f32; 3]>,
    mut normals: Vec<[f32; 3]>,
    mut uvs: Vec<[f32; 2]>,
    mut colors: Vec<[f32; 4]>,
    mut indices: Vec<u32>,
) {
    if indices.is_empty() {
        positions = vec![[0.0; 3]; 3];
        normals.clear();
        uvs.clear();
        colors.clear();
        indices = vec![0, 1, 2];
    }
    let n = positions.len();
    normals.resize(n, [0.0, 1.0, 0.0]);
    uvs.resize(n, [0.0, 0.0]);
    colors.resize(n, [1.0; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
}

/// Starts an effect at `location` (Unreal world) with axes `axes` (columns
/// X, Y, Z, Unreal world). Returns the effect entity, or None if the class
/// is not loaded.
pub fn spawn_effect(
    commands: &mut Commands,
    library: &EffectLibrary,
    meshes: &mut Assets<Mesh>,
    class: &str,
    location: Vec3,
    axes: Mat3,
    seed: u32,
) -> Option<Entity> {
    spawn_effect_for(commands, library, meshes, class, location, axes, seed, None)
}

/// Like `spawn_effect`, with the actor LifeSpan set by the spawner (e.g.
/// Gib.SpawnTrail sets the trail's to 1.8 s).
#[allow(clippy::too_many_arguments)]
pub fn spawn_effect_for(
    commands: &mut Commands,
    library: &EffectLibrary,
    meshes: &mut Assets<Mesh>,
    class: &str,
    location: Vec3,
    axes: Mat3,
    seed: u32,
    life_span: Option<f32>,
) -> Option<Entity> {
    let options = SpawnOptions { life_span, ..default() };
    spawn_effect_with(commands, library, meshes, class, location, axes, seed, options)
}

/// How an effect is spawned beyond its class defaults.
#[derive(Clone, Copy, Default)]
pub struct SpawnOptions {
    /// Actor LifeSpan set by the spawner.
    pub life_span: Option<f32>,
    /// Kept with no particles until its owner removes it (see ParticleEffect).
    pub persistent: bool,
    /// Drawn on this render layer (e.g. the first-person weapon's).
    pub layer: Option<usize>,
}

/// `spawn_effect` with options.
#[allow(clippy::too_many_arguments)]
pub fn spawn_effect_with(
    commands: &mut Commands,
    library: &EffectLibrary,
    meshes: &mut Assets<Mesh>,
    class: &str,
    location: Vec3,
    axes: Mat3,
    seed: u32,
    options: SpawnOptions,
) -> Option<Entity> {
    let SpawnOptions { life_span, persistent, layer } = options;
    let layers = bevy::camera::visibility::RenderLayers::layer(layer.unwrap_or(0));
    static NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    // Unreal names ignore case (a class reference may come back as
    // "roeffects.MuzzleFlash1stMP").
    let effect = match library.0.get(class) {
        Some(e) => e.clone(),
        None => library.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(class))?.1.clone(),
    };
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut rng = (seed ^ id.wrapping_mul(2_654_435_761)) | 1;
    let mut states = Vec::new();
    let parent = commands.spawn((Transform::IDENTITY, Visibility::Visible)).id();
    for e in &effect.emitters {
        let mut handles = Vec::new();
        let materials: Vec<SpriteMaterial> = match e.def.kind {
            EmitterKind::Sprite => e.material.iter().cloned().collect(),
            EmitterKind::Mesh => e.mesh.iter().map(|s| SpriteMaterial::Standard(s.material.clone())).collect(),
            _ => Vec::new(),
        };
        for material in materials {
            let mesh = empty_mesh(meshes);
            let common = (Mesh3d(mesh.clone()), Transform::IDENTITY, NoFrustumCulling, bevy::light::NotShadowCaster, ChildOf(parent));
            match material {
                SpriteMaterial::Standard(m) => commands.spawn((common, MeshMaterial3d(m), layers.clone())),
                SpriteMaterial::Modulate(m) => commands.spawn((common, MeshMaterial3d(m), layers.clone())),
            };
            handles.push(mesh);
        }
        states.push(EmitterState {
            slots: Vec::new(),
            next: 0,
            rate: 0.0,
            delay: in_range(&mut rng, e.def.initial_delay_range),
            spawned: 0,
            carry: 0.0,
            meshes: handles,
            pending: 0,
            start_velocity: None,
            lifetime: None,
        });
    }
    runlog::kv(
        "effect_spawned",
        &format!(
            "effect={id} class={class} at_unreal=({:.0}, {:.0}, {:.0}) x_axis=({:.2}, {:.2}, {:.2})",
            location.x,
            location.y,
            location.z,
            axes.col(0).x,
            axes.col(0).y,
            axes.col(0).z
        ),
    );
    commands.entity(parent).insert(ParticleEffect {
        persistent,
        frame: (location, axes),
        id,
        age: 0.0,
        emitters: states,
        rng,
        log_timer: 0.0,
        life_span: life_span.unwrap_or(effect.life_span),
        effect,
        killed: false,
    });
    Some(parent)
}

/// Piecewise-linear lookup in (time, value) keys; before the first key the
/// first value, after the last the last (assumed; the engine's exact rule
/// is native).
fn curve<T: Copy>(keys: &[(f32, T)], t: f32, lerp: impl Fn(T, T, f32) -> T) -> Option<T> {
    let first = keys.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    for w in keys.windows(2) {
        if t <= w[1].0 {
            let span = (w[1].0 - w[0].0).max(1e-6);
            return Some(lerp(w[0].1, w[1].1, (t - w[0].0) / span));
        }
    }
    keys.last().map(|k| k.1)
}

/// Spawn rate (particles per second), as KF's engine picks it: while fewer
/// slots than MaxParticles have ever been used, MaxParticles / average
/// lifetime with AutomaticInitialSpawning, else InitialParticlesPerSecond
/// (so with neither, nothing spawns by itself); after that
/// ParticlesPerSecond, whether or not dead particles respawn. 0 once killed.
fn spawn_rate(d: &EmitterDef, s: &EmitterState, killed: bool) -> f32 {
    let max = d.max_particles.max(0) as usize;
    if killed {
        0.0
    } else if s.slots.len() < max {
        if d.automatic_initial_spawning {
            let average = (d.lifetime.0 + d.lifetime.1) * 0.5;
            if average > 0.0 { max as f32 / average } else { 0.0 }
        } else {
            d.initial_particles_per_second
        }
    } else {
        d.particles_per_second
    }
}

/// How many particles a rate gives this frame: whole particles of rate x dt
/// plus the fraction carried over, at most MaxParticles.
fn spawn_count(d: &EmitterDef, s: &mut EmitterState, rate: f32, dt: f32) -> u32 {
    if rate <= 0.0 {
        return 0;
    }
    s.carry += rate * dt;
    let n = s.carry.floor();
    s.carry -= n;
    (n as u32).min(d.max_particles.max(0) as u32)
}

/// Puts a new particle in the next slot of the ring.
fn put(d: &EmitterDef, s: &mut EmitterState, p: Particle) {
    let max = d.max_particles.max(1) as usize;
    if s.next < s.slots.len() {
        s.slots[s.next] = p;
    } else {
        s.slots.push(p);
    }
    s.next = (s.next + 1) % max;
    s.spawned += 1;
}

/// Moves one sub-emitter's particles on by `dt` and spawns new ones (KF's
/// rules, see `spawn_rate`). `other`: world positions of the live particles
/// of the emitter named by AddLocationFromOtherEmitter. `cast`: a level ray
/// test from one Unreal point to another, giving the hit fraction and the
/// Unreal normal. Returns true when the emitter is finished: no spawn rate,
/// no respawning, and no live particle (KF's "all particles dead").
#[allow(clippy::too_many_arguments)]
fn update_emitter(
    d: &EmitterDef,
    s: &mut EmitterState,
    frame: &(Vec3, Mat3),
    dt: f32,
    killed: bool,
    other: Option<&[Vec3]>,
    rng: &mut u32,
    cast: &mut dyn FnMut(Vec3, Vec3) -> Option<(f32, Vec3)>,
) -> bool {
    // InitialDelayRange: nothing happens until the delay has run out (the
    // update that ends it runs in full); a waiting emitter is not finished.
    if s.delay > 0.0 {
        s.delay -= dt;
        if s.delay > 0.0 {
            return false;
        }
        s.delay = 0.0;
    }
    let rate = spawn_rate(d, s, killed);
    s.rate = rate;
    // Kill() also stops respawning.
    let respawn = d.respawn_dead_particles && !killed;
    // Age and move; a particle whose time is up respawns in its slot if
    // RespawnDeadParticles, else it is dead.
    let relative = d.coordinate_system == 1;
    let accel = Vec3::from_array(d.acceleration);
    let max_abs = Vec3::from_array(d.max_abs_velocity);
    for i in 0..s.slots.len() {
        let p = &mut s.slots[i];
        if !p.alive {
            continue;
        }
        p.age += dt;
        if p.age >= p.life {
            if respawn {
                let fresh = spawn_particle(d, frame, pick_other(other, rng), (s.start_velocity, s.lifetime), rng);
                s.slots[i] = fresh;
                s.spawned += 1;
            } else {
                p.alive = false;
            }
            continue;
        }
        p.vel += accel * dt;
        p.vel -= p.vel * p.velocity_loss * dt;
        for a in 0..3 {
            if max_abs[a] > 0.0 {
                p.vel[a] = p.vel[a].clamp(-max_abs[a], max_abs[a]);
            }
        }
        let step = p.vel * dt;
        if d.use_collision
            && !relative
            && step.length_squared() > 0.0
            && let Some((fraction, n)) = cast(p.pos, p.pos + step)
        {
            p.pos += step * fraction + n * 0.5;
            // Bounce: reflect, scaled per axis by DampingFactor.
            p.vel = (p.vel - 2.0 * p.vel.dot(n) * n) * p.damping;
            if d.damp_rotation {
                p.spin_rate *= p.damping;
            }
        } else {
            p.pos += step;
        }
        p.spin += p.spin_rate * dt;
    }
    // Spawn at the rate, into the ring.
    let n = spawn_count(d, s, rate, dt);
    for _ in 0..n {
        let base = match other {
            Some([]) => continue,
            o => pick_other(o, rng),
        };
        let p = spawn_particle(d, frame, base, (s.start_velocity, s.lifetime), rng);
        put(d, s, p);
    }
    // SpawnParticle / Trigger requests: into the ring too.
    let asked = std::mem::take(&mut s.pending);
    for _ in 0..asked.min(d.max_particles.max(0) as u32) {
        let p = spawn_particle(d, frame, None, (s.start_velocity, s.lifetime), rng);
        put(d, s, p);
    }
    rate == 0.0 && !respawn && !s.slots.iter().any(|p| p.alive)
}

/// A random live particle of the other emitter (AddLocationFromOtherEmitter);
/// None when there is no other emitter or it has none.
fn pick_other(other: Option<&[Vec3]>, rng: &mut u32) -> Option<Vec3> {
    let list = other.filter(|l| !l.is_empty())?;
    Some(list[(frand(rng) * list.len() as f32) as usize % list.len()])
}

/// The rotation KF gives a new particle's start position and velocity
/// (UseRotationFrom): Actor = the effect's axes (then RotationOffset;
/// the order is assumed, RotationOffset is zero in all of KF's data),
/// Offset = RotationOffset, Normal = the rotation of RotationNormal (a
/// quarter turn down for EffectAxis PositiveZ), None = world axes.
fn start_rotation(d: &EmitterDef, axes: Mat3) -> Mat3 {
    match d.use_rotation_from {
        1 => axes * coords::ue_rotation_matrix(d.rotation_offset),
        2 => coords::ue_rotation_matrix(d.rotation_offset),
        3 => {
            let [x, y, z] = d.rotation_normal;
            let to_units = 32768.0 / std::f32::consts::PI;
            let mut pitch = (z.atan2(x.hypot(y)) * to_units) as i32;
            if d.effect_axis == 1 {
                pitch -= 16384;
            }
            coords::ue_rotation_matrix(Rotator { pitch, yaw: (y.atan2(x) * to_units) as i32, roll: 0 })
        }
        _ => Mat3::IDENTITY,
    }
}

/// GetVelocityDirectionFrom, applied after turning: `dir` is the unit
/// direction from the new particle to the effect (for Relative, from the
/// effect to the particle, as KF uses the local position). KF multiplies
/// axis by axis: StartPositionAndOwner gives -(vel x dir), OwnerAndStartPosition
/// vel x dir; AddRadial adds StartVelocityRadialRange along dir.
fn velocity_direction(d: &EmitterDef, origin: Vec3, pos: Vec3, vel: Vec3, rng: &mut u32) -> Vec3 {
    if d.get_velocity_direction_from == 0 {
        return vel;
    }
    let dir = if d.coordinate_system == 1 { pos } else { origin - pos }.normalize_or_zero();
    match d.get_velocity_direction_from {
        1 => -(vel * dir),
        2 => vel * dir,
        3 => vel + dir * in_range(rng, d.start_velocity_radial_range),
        _ => vel,
    }
}

/// `base`: world position of another emitter's particle to start from
/// (AddLocationFromOtherEmitter). `start`: StartVelocityRange and
/// LifetimeRange as a script last set them.
fn spawn_particle(d: &EmitterDef, frame: &(Vec3, Mat3), base: Option<Vec3>, start: (Option<Vec3>, Option<f32>), rng: &mut u32) -> Particle {
    // Start location: StartLocationOffset, plus a box, plus a sphere shell
    // for Sphere / All.
    let mut offset = Vec3::from_array(d.start_location_offset) + in_ranges(rng, &d.start_location_range);
    if matches!(d.start_location_shape, 1 | 3) {
        let dir = loop {
            let v = Vec3::new(frand(rng) * 2.0 - 1.0, frand(rng) * 2.0 - 1.0, frand(rng) * 2.0 - 1.0);
            if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
                break v.normalize();
            }
        };
        offset += dir * in_range(rng, d.sphere_radius_range);
    }
    // Another emitter's particle (AddLocationFromOtherEmitter): its offset
    // from the effect, added before turning, as KF does.
    if let Some(b) = base {
        offset += b - frame.0;
    }
    // KF turns the start position and velocity only by UseRotationFrom
    // (None = world axes), whatever the coordinate system; then an
    // Independent particle gets the effect's location added, a Relative one
    // stays local (drawn with the effect's location and rotation, so
    // Relative + Actor turns twice, as in KF), an Absolute one is used as is.
    let rot = start_rotation(d, frame.1);
    let vel = rot * start.0.unwrap_or_else(|| in_ranges(rng, &d.start_velocity_range));
    let offset = rot * offset;
    let pos = if d.coordinate_system == 0 { frame.0 + offset } else { offset };
    let vel = velocity_direction(d, frame.0, pos, vel, rng);
    let mut velocity_loss = in_ranges(rng, &d.velocity_loss_range);
    if d.rotate_velocity_loss_range {
        velocity_loss = rot * velocity_loss;
    }
    let mut size = in_ranges(rng, &d.start_size_range);
    if d.uniform_size {
        size = Vec3::splat(size.x);
    }
    let mut spin_rate = in_ranges(rng, &d.spins_per_second_range);
    // SpinCCWorCW 0.5: either direction.
    for a in 0..3 {
        if frand(rng) < 0.5 {
            spin_rate[a] = -spin_rate[a];
        }
    }
    let subdivisions = (d.texture_u_subdivisions.max(1) * d.texture_v_subdivisions.max(1)) as u32;
    Particle {
        alive: true,
        pos,
        vel,
        size,
        age: 0.0,
        life: start.1.unwrap_or_else(|| in_range(rng, d.lifetime)).max(0.01),
        spin: if d.spin_particles { in_ranges(rng, &d.start_spin_range) } else { Vec3::ZERO },
        spin_rate: if d.spin_particles { spin_rate } else { Vec3::ZERO },
        damping: in_ranges(rng, &d.damping_factor_range),
        velocity_loss,
        subdivision: if d.use_random_subdivision { (frand(rng) * subdivisions as f32) as u32 % subdivisions } else { 0 },
    }
}

fn world_pos(d: &EmitterDef, frame: &(Vec3, Mat3), p: &Particle) -> Vec3 {
    if d.coordinate_system == 1 { frame.0 + frame.1 * p.pos } else { p.pos }
}

#[allow(clippy::too_many_arguments)]
fn update_effects(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    camera: Query<&Transform, With<FlyCamera>>,
    mut effects: Query<(Entity, &mut ParticleEffect)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let dt = time.delta_secs().min(0.1);
    let Ok(cam) = camera.single() else {
        return;
    };
    let cam_right = to_ue_dir(cam.rotation * Vec3::X);
    let cam_up = to_ue_dir(cam.rotation * Vec3::Y);
    let cam_forward = to_ue_dir(cam.rotation * Vec3::NEG_Z);
    for (entity, mut fx) in &mut effects {
        let fx = &mut *fx;
        fx.age += dt;
        fx.log_timer += dt;
        let effect = fx.effect.clone();
        let frame = fx.frame;
        // World positions of every emitter's live particles, for
        // AddLocationFromOtherEmitter.
        let snapshot: Vec<Vec<Vec3>> = effect
            .emitters
            .iter()
            .zip(&fx.emitters)
            .map(|(e, s)| s.slots.iter().filter(|p| p.alive).map(|p| world_pos(&e.def, &frame, p)).collect())
            .collect();
        let mut all_done = true;
        for (i, e) in effect.emitters.iter().enumerate() {
            let d = &e.def;
            let s = &mut fx.emitters[i];
            if d.disabled {
                continue;
            }
            let other = usize::try_from(d.add_location_from_other_emitter).ok().and_then(|o| snapshot.get(o)).map(|v| v.as_slice());
            let mut cast = |from: Vec3, to: Vec3| -> Option<(f32, Vec3)> {
                let (a, b) = (coords::pos(from.to_array()), coords::pos(to.to_array()));
                let dir = Dir3::new(b - a).ok()?;
                let hit = spatial.cast_ray(a, dir, (b - a).length(), true, &crate::world::collision::world_filter())?;
                Some((hit.distance / (b - a).length(), to_ue_dir(hit.normal).normalize_or_zero()))
            };
            let finished = update_emitter(d, s, &frame, dt, fx.killed, other, &mut fx.rng, &mut cast);
            if !finished {
                all_done = false;
            }
            let live: Vec<Particle> = s.slots.iter().filter(|p| p.alive).copied().collect();
            // Draw.
            match d.kind {
                EmitterKind::Sprite => build_sprites(d, &frame, &live, (cam_right, cam_up, cam_forward), &s.meshes, &mut meshes),
                EmitterKind::Mesh => build_mesh_particles(d, &e.mesh, &frame, &live, &s.meshes, &mut meshes),
                _ => {}
            }
        }
        let live_count = |s: &EmitterState| s.slots.iter().filter(|p| p.alive).count();
        let alive: usize = fx.emitters.iter().map(live_count).sum();
        if (fx.log_timer >= 0.5 || fx.age <= dt) && !(fx.persistent && alive == 0) {
            fx.log_timer = 0.0;
            let counts: Vec<String> = fx.emitters.iter().map(|s| format!("{}/{}@{:.1}", live_count(s), s.spawned, s.rate)).collect();
            let first = fx
                .emitters
                .iter()
                .find_map(|s| s.slots.iter().find(|p| p.alive))
                .map_or("none".to_string(), |p| format!("({:.1}, {:.1}, {:.1})", p.pos.x, p.pos.y, p.pos.z));
            runlog::kv(
                "effect_status",
                &format!(
                    "effect={} class={} age={:.2} alive/spawned@rate=[{}] first_particle_unreal={first} frame_unreal=({:.1}, {:.1}, {:.1})",
                    fx.id,
                    effect.class,
                    fx.age,
                    counts.join(" "),
                    fx.frame.0.x,
                    fx.frame.0.y,
                    fx.frame.0.z
                ),
            );
        }
        let expired = fx.life_span > 0.0 && fx.age > fx.life_span;
        let killed_and_empty = fx.killed && alive == 0;
        if expired || killed_and_empty || (all_done && fx.age > 0.1 && !fx.persistent) {
            let spawned: u32 = fx.emitters.iter().map(|s| s.spawned).sum();
            runlog::kv(
                "effect_removed",
                &format!(
                    "effect={} class={} age={:.2} reason={} particles_spawned={spawned}",
                    fx.id,
                    effect.class,
                    fx.age,
                    if expired {
                        "life_span"
                    } else if killed_and_empty {
                        "killed"
                    } else {
                        "all_emitters_done"
                    }
                ),
            );
            commands.entity(entity).despawn();
        }
    }
}

/// A sprite's four corners (Unreal units): KF puts them at centre +- right
/// x Size.X +- up x Size.Y, so a sprite is 2 x Size wide and tall (Size is
/// the half-width). Order: top-left, top-right, bottom-right, bottom-left.
fn sprite_corners(centre: Vec3, right: Vec3, up: Vec3, size: Vec2) -> [Vec3; 4] {
    [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)].map(|(sx, sy)| centre + right * (sx * size.x) + up * (sy * size.y))
}

/// Sprites: one quad per particle, 2 x size wide (see `sprite_corners`), facing the
/// camera (UseDirectionAs None), with its up (Up) or right (Right) along the
/// velocity, or, for UpAndNormal, stretched along the velocity in the plane
/// of ProjectionNormal.
fn build_sprites(
    d: &EmitterDef,
    frame: &(Vec3, Mat3),
    particles: &[Particle],
    (cam_right, cam_up, cam_forward): (Vec3, Vec3, Vec3),
    handles: &[Handle<Mesh>],
    meshes: &mut Assets<Mesh>,
) {
    let Some(mut mesh) = handles.first().and_then(|h| meshes.get_mut(h)) else {
        return;
    };
    let (su, sv) = (d.texture_u_subdivisions.max(1) as u32, d.texture_v_subdivisions.max(1) as u32);
    let frames = su * sv;
    let (mut positions, mut uvs, mut colors, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for p in particles {
        let t = (p.age / p.life).clamp(0.0, 1.0);
        // Width and height: Size.X for both with UniformSize, else X and Y.
        let mut size = if d.uniform_size { Vec2::splat(p.size.x) } else { Vec2::new(p.size.x, p.size.y) };
        if d.use_size_scale
            && let Some(k) = curve(&d.size_scale, t, |a, b, f| a + (b - a) * f)
        {
            size *= k;
        }
        // ScaleSizeXByVelocity / Y: size x speed x ScaleSizeByVelocityMultiplier,
        // at least 1x and at most ScaleSizeByVelocityMax (assumed: the
        // native code is not in the scripts). Makes tracers long streaks.
        let speed = p.vel.length();
        for a in 0..2 {
            if d.scale_size_by_velocity[a] {
                size[a] *= (speed * d.scale_size_by_velocity_multiplier[a]).clamp(1.0, d.scale_size_by_velocity_max.max(1.0));
            }
        }
        // Fading, in seconds of the particle's life.
        let mut alpha = d.opacity;
        if d.fade_in && d.fade_in_end_time > 0.0 && p.age < d.fade_in_end_time {
            alpha *= p.age / d.fade_in_end_time;
        }
        if d.fade_out && p.age > d.fade_out_start_time {
            alpha *= 1.0 - (p.age - d.fade_out_start_time) / (p.life - d.fade_out_start_time).max(1e-3);
        }
        let mut rgb = Vec3::ONE;
        if d.use_color_scale
            && let Some(c) = curve(
                &d.color_scale.iter().map(|(t, c)| (*t, Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32) / 255.0)).collect::<Vec<_>>(),
                t,
                |a, b, f| a.lerp(b, f),
            )
        {
            rgb = c.truncate();
        }
        let centre = world_pos(d, frame, p);
        let (right, up) = if d.use_direction_as == 2 && p.vel.length_squared() > 1e-6 {
            // PTDU_Right: the sprite's right (its width) along the velocity,
            // facing the camera (KFNewTracer).
            let vel = if d.coordinate_system == 1 { frame.1 * p.vel } else { p.vel };
            let right = vel.normalize();
            let up = right.cross(cam_forward).normalize_or(cam_up);
            (right, up)
        } else if d.use_direction_as == 1 && p.vel.length_squared() > 1e-6 {
            // PTDU_Up: the sprite's up along the velocity, facing the camera.
            let vel = if d.coordinate_system == 1 { frame.1 * p.vel } else { p.vel };
            let up = vel.normalize();
            let right = up.cross(cam_forward).normalize_or(cam_right);
            (right, up)
        } else if d.use_direction_as == 5 && p.vel.length_squared() > 1e-6 {
            let vel = if d.coordinate_system == 1 { frame.1 * p.vel } else { p.vel };
            let normal = if d.coordinate_system == 1 { frame.1 * Vec3::from_array(d.projection_normal) } else { Vec3::from_array(d.projection_normal) };
            let up = vel.normalize();
            let right = up.cross(normal).normalize_or(cam_right);
            (right, up)
        } else {
            // Facing the camera, turned by the particle's spin.
            let angle = p.spin.x * std::f32::consts::TAU;
            let q = Quat::from_axis_angle(cam_forward.normalize_or(Vec3::X), angle);
            (q * cam_right, q * cam_up)
        };
        let base = positions.len() as u32;
        for corner in sprite_corners(centre, right, up, size) {
            positions.push(coords::pos(corner.to_array()).to_array());
        }
        // Texture subdivision: by age unless random (BlendBetweenSubdivisions
        // is not blended here).
        let index = if d.use_random_subdivision { p.subdivision } else { ((t * frames as f32) as u32).min(frames - 1) };
        let (cu, cv) = ((index % su) as f32, (index / su) as f32);
        let (w, hgt) = (1.0 / su as f32, 1.0 / sv as f32);
        uvs.extend([[cu * w, cv * hgt], [(cu + 1.0) * w, cv * hgt], [(cu + 1.0) * w, (cv + 1.0) * hgt], [cu * w, (cv + 1.0) * hgt]]);
        colors.extend([[rgb.x, rgb.y, rgb.z, alpha.clamp(0.0, 1.0)]; 4]);
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    write_mesh(&mut mesh, positions, Vec::new(), uvs, colors, indices);
}

/// Mesh particles: the mesh copied per particle, scaled by its size and
/// turned by its spin (X/Y/Z revolutions as yaw/pitch/roll; assumed).
fn build_mesh_particles(
    d: &EmitterDef,
    sections: &[MeshSection],
    frame: &(Vec3, Mat3),
    particles: &[Particle],
    handles: &[Handle<Mesh>],
    meshes: &mut Assets<Mesh>,
) {
    for (section, handle) in sections.iter().zip(handles) {
        let Some(mut mesh) = meshes.get_mut(handle) else {
            continue;
        };
        let (mut positions, mut normals, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for p in particles {
            let centre = world_pos(d, frame, p);
            let r = coords::ue_rotation_matrix(Rotator {
                pitch: (p.spin.y * 65536.0) as i32,
                yaw: (p.spin.x * 65536.0) as i32,
                roll: (p.spin.z * 65536.0) as i32,
            });
            let base = positions.len() as u32;
            for (v, n) in section.positions.iter().zip(&section.normals) {
                positions.push(coords::pos((centre + r * (*v * p.size)).to_array()).to_array());
                normals.push(to_bevy_dir(r * *n).normalize_or_zero().to_array());
            }
            uvs.extend_from_slice(&section.uvs);
            indices.extend(section.indices.iter().map(|i| base + i));
        }
        write_mesh(&mut mesh, positions, normals, uvs, Vec::new(), indices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sub-emitter with the engine's defaults (as in our reader's
    /// fallbacks), for tests.
    fn def() -> EmitterDef {
        EmitterDef {
            name: "test".into(),
            kind: EmitterKind::Sprite,
            max_particles: 10,
            respawn_dead_particles: false,
            automatic_initial_spawning: false,
            initial_particles_per_second: 0.0,
            particles_per_second: 0.0,
            lifetime: (4.0, 4.0),
            initial_delay_range: (0.0, 0.0),
            seconds_before_inactive: 1.0,
            reset_after_change: false,
            start_location_offset: [0.0; 3],
            start_location_range: [(0.0, 0.0); 3],
            start_location_shape: 0,
            sphere_radius_range: (0.0, 0.0),
            add_location_from_other_emitter: -1,
            coordinate_system: 0,
            start_velocity_range: [(0.0, 0.0); 3],
            get_velocity_direction_from: 0,
            start_velocity_radial_range: (0.0, 0.0),
            velocity_loss_range: [(0.0, 0.0); 3],
            max_abs_velocity: [0.0; 3],
            acceleration: [0.0; 3],
            use_collision: false,
            damping_factor_range: [(1.0, 1.0); 3],
            use_velocity_scale: false,
            velocity_scale: Vec::new(),
            start_size_range: [(100.0, 100.0); 3],
            uniform_size: false,
            use_size_scale: false,
            use_regular_size_scale: false,
            size_scale: Vec::new(),
            scale_size_by_velocity_multiplier: [1.0; 3],
            scale_size_by_velocity_max: 10_000_000.0,
            scale_size_by_velocity: [false; 3],
            use_color_scale: false,
            color_scale: Vec::new(),
            opacity: 1.0,
            fade_in: false,
            fade_in_end_time: 0.0,
            fade_out: false,
            fade_out_start_time: 0.0,
            spin_particles: false,
            start_spin_range: [(0.0, 0.0); 3],
            spins_per_second_range: [(0.0, 0.0); 3],
            damp_rotation: false,
            use_rotation_from: 0,
            rotation_offset: Rotator::default(),
            rotation_normal: [0.0; 3],
            effect_axis: 0,
            rotate_velocity_loss_range: false,
            draw_style: 1,
            use_direction_as: 0,
            projection_normal: [0.0, 0.0, 1.0],
            texture: None,
            texture_u_subdivisions: 0,
            texture_v_subdivisions: 0,
            blend_between_subdivisions: false,
            use_random_subdivision: false,
            static_mesh: None,
            disabled: false,
            trigger_disabled: false,
            reset_on_trigger: false,
            spawn_on_trigger: (0.0, 0.0),
            spawn_on_trigger_pps: 0.0,
        }
    }

    /// The effect turned a quarter turn (its X axis along world Y).
    fn turned_frame() -> (Vec3, Mat3) {
        (Vec3::new(100.0, 0.0, 0.0), coords::ue_rotation_matrix(Rotator { pitch: 0, yaw: 16384, roll: 0 }))
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }

    /// UseRotationFrom None: start offset and velocity stay in world axes
    /// however the effect is turned.
    #[test]
    fn rotation_none_keeps_world_axes() {
        let mut d = def();
        d.start_location_range = [(10.0, 10.0), (0.0, 0.0), (0.0, 0.0)];
        d.start_velocity_range = [(0.0, 0.0), (0.0, 0.0), (50.0, 50.0)];
        d.velocity_loss_range = [(1.0, 1.0), (0.0, 0.0), (0.0, 0.0)];
        d.rotate_velocity_loss_range = true;
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.vel, Vec3::new(0.0, 0.0, 50.0)));
        assert!(close(p.pos, Vec3::new(110.0, 0.0, 0.0)));
        assert!(close(p.velocity_loss, Vec3::X));
    }

    /// UseRotationFrom Actor: turned with the effect (X along world Y).
    #[test]
    fn rotation_actor_turns_with_effect() {
        let mut d = def();
        d.use_rotation_from = 1;
        d.start_location_range = [(10.0, 10.0), (0.0, 0.0), (0.0, 0.0)];
        d.start_velocity_range = [(50.0, 50.0), (0.0, 0.0), (0.0, 0.0)];
        d.velocity_loss_range = [(1.0, 1.0), (0.0, 0.0), (0.0, 0.0)];
        d.rotate_velocity_loss_range = true;
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.vel, Vec3::new(0.0, 50.0, 0.0)));
        assert!(close(p.pos, Vec3::new(100.0, 10.0, 0.0)));
        assert!(close(p.velocity_loss, Vec3::Y));
        // Relative: kept local but turned (drawn turned again, as in KF).
        d.coordinate_system = 1;
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.pos, Vec3::new(0.0, 10.0, 0.0)));
    }

    /// UseRotationFrom Normal: the rotation of RotationNormal.
    #[test]
    fn rotation_normal() {
        let mut d = def();
        d.use_rotation_from = 3;
        d.rotation_normal = [0.0, 1.0, 0.0];
        d.start_velocity_range = [(50.0, 50.0), (0.0, 0.0), (0.0, 0.0)];
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.vel, Vec3::new(0.0, 50.0, 0.0)), "{:?}", p.vel);
        d.rotation_normal = [0.0; 3];
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.vel, Vec3::new(50.0, 0.0, 0.0)));
    }

    fn state() -> EmitterState {
        EmitterState {
            slots: Vec::new(),
            next: 0,
            pending: 0,
            start_velocity: None,
            lifetime: None,
            spawned: 0,
            carry: 0.0,
            rate: 0.0,
            delay: 0.0,
            meshes: Vec::new(),
        }
    }

    /// Runs one emitter for `seconds` at 20 updates a second; returns
    /// (finished, live particles) at the end.
    fn run(d: &EmitterDef, s: &mut EmitterState, seconds: f32) -> (bool, usize) {
        let frame = (Vec3::ZERO, Mat3::IDENTITY);
        let mut rng = 12345;
        let mut finished = false;
        for _ in 0..(seconds * 20.0).round() as usize {
            finished = update_emitter(d, s, &frame, 0.05, false, None, &mut rng, &mut |_, _| None);
        }
        (finished, s.slots.iter().filter(|p| p.alive).count())
    }

    /// ROEffects FireLarge: InitialParticlesPerSecond 5, ParticlesPerSecond
    /// 5, no respawning: it keeps burning at 5 a second (it burnt out
    /// after one batch of 10 before).
    #[test]
    fn steady_rate_without_respawn_keeps_spawning() {
        let mut d = def();
        d.initial_particles_per_second = 5.0;
        d.particles_per_second = 5.0;
        d.lifetime = (1.0, 1.15);
        let mut s = state();
        let (finished, live) = run(&d, &mut s, 10.0);
        assert!(!finished);
        assert!((49..=51).contains(&s.spawned), "spawned {}", s.spawned);
        assert!((5..=7).contains(&live), "live {live}");
        assert_eq!(s.slots.len(), 10);
    }

    /// AutomaticInitialSpawning wins over InitialParticlesPerSecond: rate
    /// MaxParticles / average lifetime (ZEDProjectileTrail's glow: 1 / 0.1 s).
    #[test]
    fn automatic_rate_wins() {
        let mut d = def();
        d.max_particles = 1;
        d.automatic_initial_spawning = true;
        d.initial_particles_per_second = 1.0;
        d.lifetime = (0.1, 0.1);
        let mut s = state();
        run(&d, &mut s, 0.1);
        assert_eq!(s.spawned, 1);
        run(&d, &mut s, 0.05);
        assert_eq!(s.rate, 0.0, "after the first slot is used: ParticlesPerSecond 0");
    }

    /// A burst: InitialParticlesPerSecond fills the slots once, then
    /// nothing (ParticlesPerSecond 0, no respawn), and it finishes when the
    /// last one dies. With no rate at all it never spawns.
    #[test]
    fn burst_then_finished() {
        let mut d = def();
        d.max_particles = 5;
        d.initial_particles_per_second = 100.0;
        d.lifetime = (1.0, 1.0);
        let mut s = state();
        assert_eq!(run(&d, &mut s, 0.1), (false, 5));
        assert_eq!(run(&d, &mut s, 2.0), (true, 0));
        assert_eq!(s.spawned, 5);
        d.initial_particles_per_second = 0.0;
        let mut s = state();
        assert_eq!(run(&d, &mut s, 1.0), (true, 0));
        assert_eq!(s.spawned, 0);
    }

    /// RespawnDeadParticles: a dead particle comes back in its slot.
    #[test]
    fn respawn_in_place() {
        let mut d = def();
        d.max_particles = 3;
        d.initial_particles_per_second = 100.0;
        d.respawn_dead_particles = true;
        d.lifetime = (0.5, 0.5);
        let mut s = state();
        let (finished, live) = run(&d, &mut s, 5.0);
        assert!(!finished);
        assert_eq!((live, s.slots.len()), (3, 3));
        assert!(s.spawned >= 27, "spawned {}", s.spawned);
    }

    /// SpawnParticle(n): at most MaxParticles per update (as KF clamps
    /// them), into the ring, which then reuses the oldest slots.
    #[test]
    fn requests_go_into_the_ring() {
        let mut d = def();
        d.max_particles = 2;
        let mut s = state();
        s.pending = 3;
        run(&d, &mut s, 0.05);
        assert_eq!((s.slots.len(), s.spawned, s.next), (2, 2, 0));
        s.pending = 1;
        run(&d, &mut s, 0.05);
        assert_eq!((s.slots.len(), s.spawned, s.next), (2, 3, 1));
    }

    /// GetVelocityDirectionFrom with ROBloodPuff's numbers (start 20 along
    /// X, velocity -100 along X, UseRotationFrom Actor) on an effect turned
    /// a quarter turn: OwnerAndStartPosition makes the blood fly away from
    /// the effect, along its turned X (world Y).
    #[test]
    fn velocity_direction_modes() {
        let mut d = def();
        d.use_rotation_from = 1;
        d.start_location_range = [(20.0, 20.0), (0.0, 0.0), (0.0, 0.0)];
        d.start_velocity_range = [(-100.0, -100.0), (0.0, 0.0), (0.0, 0.0)];
        d.get_velocity_direction_from = 2;
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.vel, Vec3::new(0.0, 100.0, 0.0)), "{:?}", p.vel);
        d.get_velocity_direction_from = 1;
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.vel, Vec3::new(0.0, -100.0, 0.0)), "{:?}", p.vel);
        d.get_velocity_direction_from = 3;
        d.start_velocity_range = [(0.0, 0.0); 3];
        d.start_velocity_radial_range = (50.0, 50.0);
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.vel, Vec3::new(0.0, -50.0, 0.0)), "{:?}", p.vel);
    }

    /// InitialDelayRange: no particles before the delay, then spawning.
    #[test]
    fn initial_delay() {
        let mut d = def();
        d.initial_particles_per_second = 100.0;
        let mut s = state();
        s.delay = 0.52;
        assert_eq!(run(&d, &mut s, 0.5), (false, 0));
        assert_eq!(run(&d, &mut s, 0.05).1, 5);
    }

    /// StartLocationOffset (KFLawMuzzFlash-like 5 along X): added before
    /// turning, so with UseRotationFrom Actor it follows the effect's X.
    #[test]
    fn start_location_offset() {
        let mut d = def();
        d.start_location_offset = [5.0, 0.0, 0.0];
        d.start_location_range = [(0.0, 0.0), (0.0, 0.0), (1.0, 1.0)];
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.pos, Vec3::new(105.0, 0.0, 1.0)), "{:?}", p.pos);
        d.use_rotation_from = 1;
        let p = spawn_particle(&d, &turned_frame(), None, (None, None), &mut 7);
        assert!(close(p.pos, Vec3::new(100.0, 5.0, 1.0)), "{:?}", p.pos);
    }

    /// KF's sprites are 2 x Size across: Size 10 gives corners 20 apart.
    #[test]
    fn sprite_is_twice_size_wide() {
        let c = sprite_corners(Vec3::new(5.0, 0.0, 0.0), Vec3::Y, Vec3::Z, Vec2::splat(10.0));
        assert!(((c[1] - c[0]).length() - 20.0).abs() < 1e-5);
        assert!(((c[1] - c[2]).length() - 20.0).abs() < 1e-5);
        assert_eq!(c[0], Vec3::new(5.0, -10.0, 10.0));
    }
}
