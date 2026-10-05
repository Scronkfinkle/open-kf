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

use crate::camera::FlyCamera;
use crate::coords;
use crate::map::MapRequest;
use crate::runlog;

/// Effects loaded at startup (the gore effects, see DESIGN.md).
const EFFECT_CLASSES: [&str; 71] = [
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
];

pub struct ParticlePlugin;

impl Plugin for ParticlePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ModulateMaterial>::default())
            .add_systems(PostStartup, load_library)
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

struct EmitterState {
    particles: Vec<Particle>,
    /// Asked for by SpawnParticle / Trigger, spawned on the next update.
    pending: u32,
    /// StartVelocityRange / LifetimeRange as last set by a script.
    start_velocity: Option<Vec3>,
    lifetime: Option<f32>,
    /// Particles spawned so far, and the fractional spawn carry-over.
    spawned: u32,
    carry: f32,
    /// One mesh per drawn section.
    meshes: Vec<Handle<Mesh>>,
}

#[derive(Clone, Copy)]
struct Particle {
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
                    let image = assets.texture.as_ref().and_then(|t| {
                        let m = ue_assets::material::resolve(&set, t, ObjectRef::Export(t.export));
                        m.texture.as_ref().and_then(|tex| decode(tex, opaque, d.draw_style == 2, &mut images))
                    });
                    notes.push(format!("{}:sprite:texture={}", d.name, image.is_some()));
                    loaded.material = Some(match (d.draw_style, image) {
                        (2, Some(texture)) => SpriteMaterial::Modulate(modulate.add(ModulateMaterial { texture })),
                        (_, image) => SpriteMaterial::Standard(materials.add(StandardMaterial {
                            base_color_texture: image,
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
                        .map(|h| load_mesh(&set, h, &mut images, &mut materials))
                        .unwrap_or_default();
                    notes.push(format!("{}:mesh:sections={}", d.name, loaded.mesh.len()));
                }
                _ => notes.push(format!("{}:{:?}:not_drawn", d.name, d.kind)),
            }
            emitters.push(loaded);
        }
        runlog::kv("effect_loaded", &format!("class={class} life_span={} emitters=[{}]", effect.life_span, notes.join(" ")));
        library.0.insert(
            class.to_string(),
            Arc::new(LoadedEffect {
                class: class.to_string(),
                emitters,
                life_span: effect.life_span,
            }),
        );
    }
    runlog::kv("effects_ready", &format!("count={} seconds={:.2}", library.0.len(), started.elapsed().as_secs_f64()));
    commands.insert_resource(library);
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
        let image = simple.texture.as_ref().and_then(|t| crate::skinned::decode_image(t, images));
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
            particles: Vec::new(),
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
        rng: (seed ^ id.wrapping_mul(2_654_435_761)) | 1,
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

/// How many particles to spawn this frame (UE2 spawning rules, assumed from
/// the settings' meaning): InitialParticlesPerSecond until MaxParticles
/// have been spawned, then ParticlesPerSecond (0 = MaxParticles over the
/// longest lifetime) if dead particles respawn. AutomaticInitialSpawning
/// uses that automatic rate for the initial phase too.
fn spawn_count(d: &EmitterDef, s: &mut EmitterState, dt: f32) -> u32 {
    let max = d.max_particles.max(0) as u32;
    let alive = s.particles.len() as u32;
    let auto_rate = max as f32 / d.lifetime.1.max(0.01);
    let initial = s.spawned < max;
    let rate = if initial {
        if d.initial_particles_per_second > 0.0 {
            d.initial_particles_per_second
        } else if d.automatic_initial_spawning {
            auto_rate
        } else {
            d.particles_per_second
        }
    } else if d.respawn_dead_particles {
        if d.particles_per_second > 0.0 { d.particles_per_second } else { auto_rate }
    } else {
        0.0
    };
    s.carry += rate * dt;
    let mut n = s.carry.floor() as u32;
    s.carry -= n as f32;
    n = n.min(max.saturating_sub(alive));
    if !d.respawn_dead_particles {
        n = n.min(max.saturating_sub(s.spawned));
    }
    n
}

/// `start`: StartVelocityRange and LifetimeRange as a script last set them.
fn spawn_particle(d: &EmitterDef, frame: &(Vec3, Mat3), base: Option<Vec3>, start: (Option<Vec3>, Option<f32>), rng: &mut u32) -> Particle {
    // Start location: a box, plus a sphere shell for Sphere / All.
    let mut offset = in_ranges(rng, &d.start_location_range);
    if matches!(d.start_location_shape, 1 | 3) {
        let dir = loop {
            let v = Vec3::new(frand(rng) * 2.0 - 1.0, frand(rng) * 2.0 - 1.0, frand(rng) * 2.0 - 1.0);
            if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
                break v.normalize();
            }
        };
        offset += dir * in_range(rng, d.sphere_radius_range);
    }
    let relative = d.coordinate_system == 1;
    // Relative: kept in the effect's frame. Otherwise world: the offset and
    // the start velocity are turned with the effect, then the particle
    // moves in world space (assumed from KF's data: KFVomitJet's notify
    // turns the effect with OffsetRotation and its spray flies along X).
    let vel = start.0.unwrap_or_else(|| in_ranges(rng, &d.start_velocity_range));
    let vel = if relative { vel } else { frame.1 * vel };
    let pos = match (relative, base) {
        (_, Some(b)) => b + offset,
        (true, None) => offset,
        (false, None) => frame.0 + frame.1 * offset,
    };
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
        pos,
        vel,
        size,
        age: 0.0,
        life: start.1.unwrap_or_else(|| in_range(rng, d.lifetime)).max(0.01),
        spin: if d.spin_particles { in_ranges(rng, &d.start_spin_range) } else { Vec3::ZERO },
        spin_rate: if d.spin_particles { spin_rate } else { Vec3::ZERO },
        damping: in_ranges(rng, &d.damping_factor_range),
        velocity_loss: in_ranges(rng, &d.velocity_loss_range),
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
            .map(|(e, s)| s.particles.iter().map(|p| world_pos(&e.def, &frame, p)).collect())
            .collect();
        let mut all_done = true;
        for (i, e) in effect.emitters.iter().enumerate() {
            let d = &e.def;
            let s = &mut fx.emitters[i];
            if d.disabled {
                continue;
            }
            // Age and move.
            let relative = d.coordinate_system == 1;
            let accel = Vec3::from_array(d.acceleration);
            let max_abs = Vec3::from_array(d.max_abs_velocity);
            s.particles.retain_mut(|p| {
                p.age += dt;
                if p.age >= p.life {
                    return false;
                }
                p.vel += accel * dt;
                p.vel -= p.vel * p.velocity_loss * dt;
                for a in 0..3 {
                    if max_abs[a] > 0.0 {
                        p.vel[a] = p.vel[a].clamp(-max_abs[a], max_abs[a]);
                    }
                }
                let step = p.vel * dt;
                if d.use_collision && !relative && step.length_squared() > 0.0 {
                    let from = coords::pos(p.pos.to_array());
                    let to = coords::pos((p.pos + step).to_array());
                    if let Ok(dir) = Dir3::new(to - from)
                        && let Some(hit) = spatial.cast_ray(from, dir, (to - from).length(), true, &crate::collision::world_filter())
                    {
                        let n = to_ue_dir(hit.normal).normalize_or_zero();
                        p.pos += step * (hit.distance / (to - from).length()) + n * 0.5;
                        // Bounce: reflect, scaled per axis by DampingFactor.
                        p.vel = (p.vel - 2.0 * p.vel.dot(n) * n) * p.damping;
                        if d.damp_rotation {
                            p.spin_rate *= p.damping;
                        }
                        p.spin += p.spin_rate * dt;
                        return true;
                    }
                }
                p.pos += step;
                p.spin += p.spin_rate * dt;
                true
            });
            // Spawn (nothing once killed).
            let n = if fx.killed { 0 } else { spawn_count(d, s, dt) };
            let other = usize::try_from(d.add_location_from_other_emitter).ok().and_then(|o| snapshot.get(o));
            for _ in 0..n {
                let base = match other {
                    Some(list) if !list.is_empty() => Some(list[(frand(&mut fx.rng) * list.len() as f32) as usize % list.len()]),
                    // Assumed: nothing to spawn on, so no particle.
                    Some(_) => continue,
                    None => None,
                };
                let p = spawn_particle(d, &frame, base, (s.start_velocity, s.lifetime), &mut fx.rng);
                s.particles.push(p);
                s.spawned += 1;
            }
            // SpawnParticle / Trigger requests: at most MaxParticles alive,
            // the oldest dropped to make room.
            let asked = std::mem::take(&mut s.pending);
            let max = d.max_particles.max(1) as usize;
            for _ in 0..asked {
                if s.particles.len() >= max {
                    s.particles.remove(0);
                }
                let p = spawn_particle(d, &frame, None, (s.start_velocity, s.lifetime), &mut fx.rng);
                s.particles.push(p);
                s.spawned += 1;
            }
            let finished = !d.respawn_dead_particles && s.spawned >= d.max_particles.max(0) as u32 && s.particles.is_empty();
            // An emitter that spawns on another's particles is done when that one is.
            let waiting_on_other = other.is_some() && s.spawned < d.max_particles.max(0) as u32;
            if !(finished || (waiting_on_other && s.particles.is_empty())) {
                all_done = false;
            }
            // Draw.
            match d.kind {
                EmitterKind::Sprite => build_sprites(d, &frame, &s.particles, (cam_right, cam_up, cam_forward), &s.meshes, &mut meshes),
                EmitterKind::Mesh => build_mesh_particles(d, &e.mesh, &frame, &s.particles, &s.meshes, &mut meshes),
                _ => {}
            }
        }
        let alive: usize = fx.emitters.iter().map(|s| s.particles.len()).sum();
        if (fx.log_timer >= 0.5 || fx.age <= dt) && !(fx.persistent && alive == 0) {
            fx.log_timer = 0.0;
            let counts: Vec<String> = fx.emitters.iter().map(|s| format!("{}/{}", s.particles.len(), s.spawned)).collect();
            let first = fx
                .emitters
                .iter()
                .find_map(|s| s.particles.first())
                .map_or("none".to_string(), |p| format!("({:.1}, {:.1}, {:.1})", p.pos.x, p.pos.y, p.pos.z));
            runlog::kv(
                "effect_status",
                &format!(
                    "effect={} class={} age={:.2} alive/spawned=[{}] first_particle_unreal={first} frame_unreal=({:.1}, {:.1}, {:.1})",
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
        let killed_and_empty = fx.killed && fx.emitters.iter().all(|s| s.particles.is_empty());
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

/// Sprites: one quad per particle, full width = size (assumed), facing the
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
        let h = size * 0.5;
        let base = positions.len() as u32;
        for (sx, sy) in [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            positions.push(coords::pos((centre + right * (sx * h.x) + up * (sy * h.y)).to_array()).to_array());
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
