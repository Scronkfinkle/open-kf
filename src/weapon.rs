//! First-person weapons: skeletal mesh + animations, CPU skinning, a separate
//! weapon camera with the weapon's own field of view (like UE2's DisplayFOV,
//! which also keeps the weapon from clipping into walls).
//!
//! Mesh loading, animation sampling and skinning live in `skinned.rs`.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::Value;

use crate::camera::FlyCamera;
use crate::coords;
use crate::map::MapRequest;
use crate::runlog;
use crate::combat::{MeleeSwing, ShotFired};
use crate::skinned::{SkinnedModel, Skins};

/// Render layer seen only by the weapon camera.
pub const WEAPON_LAYER: usize = 2;

/// The starting weapons, in slot order: key 1 = knife, key 2 = 9mm.
const WEAPON_CLASSES: [&str; 2] = ["KFMod.Knife", "KFMod.Single"];

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScriptedInput>()
            .add_systems(PostStartup, load_weapons.after(crate::camera::spawn_camera))
            .add_systems(
                Update,
                (weapon_input, animate_weapon)
                    .chain()
                    .after(crate::camera::follow_sky),
            );
    }
}

/// Scripted input for tests: at frame N do an action ("fire", "1", "2",
/// "reload", "aim" = toggle iron sights, "zed" = spawn a Clot, "zed_drop" =
/// spawn one 200 units up, "gorefast" = spawn a Gorefast,
/// "gorefast_far" = one 900 units away, "cycle_zed" = press N,
/// "spawn_<kind>" = spawn that zed, "hurt_zeds" = 100 damage to every zed).
#[derive(Resource, Default, Clone)]
pub struct ScriptedInput(pub Vec<(u32, String)>);

/// Speed bonus etc. that other systems read (walking uses `ground_speed_bonus`).
#[derive(Resource, Default)]
pub struct WeaponEffects {
    /// Added to GroundSpeed, in Unreal units/s (KFHumanPawn InventorySpeedModifier).
    pub ground_speed_bonus: f32,
}

struct WeaponDef {
    class: String,
    model: SkinnedModel,
    entities: Vec<Entity>,
    /// PlayerViewOffset, Bevy space, before the CalcDrawOffset FOV factor.
    view_offset: Vec3,
    display_fov: f32,
    /// Iron sights (bHasAimingMode); None for weapons without them.
    iron: Option<IronSights>,
    fire_anims: Vec<String>,
    fire_anim_rate: f32,
    fire_rate: f32,
    speed_bonus: f32,
    /// Inventory BobDamping (9mm 6, knife 8).
    bob_damping: f32,
    combat: CombatStats,
    /// Magazine and spare rounds, for weapons that use ammo.
    ammo: Option<Ammo>,
}

/// Iron sight values from the weapon and fire mode classes.
#[derive(Clone, Debug)]
struct IronSights {
    /// Player FOV while aiming (PlayerIronSightFOV), horizontal at 4:3.
    player_fov: f32,
    /// Weapon DisplayFOV while aiming (ZoomedDisplayFOV).
    display_fov: f32,
    /// Seconds to zoom in or out (ZoomTime).
    zoom_time: f32,
    /// Seconds for the quick zoom out when reloading or switching (FastZoomOutTime).
    fast_zoom_out_time: f32,
    /// Idle and fire animations while aiming (IdleAimAnim, FireAimedAnim).
    idle_anim: String,
    fire_anim: String,
}

/// Damage values from the fire mode and damage type classes.
#[derive(Clone, Copy, Debug, Default)]
struct CombatStats {
    melee: bool,
    damage_min: f32,
    damage_max: f32,
    spread: f32,
    headshot_mult: f32,
    range: f32,
    damage_delay: f32,
    min_dot: f32,
}

#[derive(Clone, Copy, Debug)]
struct Ammo {
    mag: u32,
    spare: u32,
    capacity: u32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Select,
    Idle,
    Fire,
    Reload,
    PutDown { next: usize },
}

#[derive(Resource)]
struct Weapons {
    defs: Vec<WeaponDef>,
    current: usize,
    action: Action,
    sequence: Option<usize>,
    /// Current time in the sequence, in frames.
    frame: f32,
    /// Frames per second for the current sequence.
    play_rate: f32,
    looping: bool,
    fire_cooldown: f32,
    fire_count: usize,
    /// Seconds until a pending knife swing deals damage.
    pending_swing: Option<f32>,
    /// Iron sights wanted (bAimingRifle) and the zoom blend, 0 = hip, 1 = aimed.
    aiming: bool,
    zoom: f32,
    /// Seconds a full zoom transition takes right now.
    zoom_time: f32,
    /// Simple random number state (spread, damage rolls).
    rng: u64,
}

impl Weapons {
    /// A random number in [0, 1) (xorshift).
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[derive(Component)]
struct WeaponCamera;

#[allow(clippy::too_many_arguments)]
fn load_weapons(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    main_cam: Query<(Entity, &Transform), With<FlyCamera>>,
) {
    let started = std::time::Instant::now();
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    let Ok((_, main_t)) = main_cam.single() else {
        return;
    };

    // Weapon camera: renders only the weapon layer, after the scene, with its
    // own depth buffer so the weapon is never hidden by walls.
    let cam = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: 1,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            Projection::from(PerspectiveProjection {
                fov: crate::camera::vertical_fov(70.0),
                near: 0.01,
                ..default()
            }),
            *main_t,
            RenderLayers::layer(WEAPON_LAYER),
            WeaponCamera,
        ))
        .id();

    let mut defs = Vec::new();
    for class_path in WEAPON_CLASSES {
        match load_weapon(&set, &defaults, class_path, &mut meshes, &mut images, &mut materials) {
            Ok(mut def) => {
                for part in &def.model.parts {
                    let e = commands
                        .spawn((
                            Mesh3d(part.mesh.clone()),
                            MeshMaterial3d(part.material.clone()),
                            Transform::from_translation(def.view_offset * draw_offset_factor(def.display_fov)),
                            RenderLayers::layer(WEAPON_LAYER),
                            Visibility::Hidden,
                            ChildOf(cam),
                        ))
                        .id();
                    def.entities.push(e);
                }
                runlog::kv(
                    "weapon_loaded",
                    &format!(
                        "class={} bones={} points={} triangles={} parts={} sequences={:?} display_fov={} view_offset_unreal={:?} speed_bonus={} combat={:?} ammo={:?} iron_sights={:?}",
                        def.class,
                        def.model.mesh.bones.len(),
                        def.model.mesh.points.len(),
                        def.model.mesh.triangles.len(),
                        def.model.parts.len(),
                        def.model.anim.as_ref().map(|a| a.sequences.iter().map(|s| s.name.clone()).collect::<Vec<_>>()),
                        def.display_fov,
                        (def.view_offset / coords::SCALE).to_array(),
                        def.speed_bonus,
                        def.combat,
                        def.ammo,
                        def.iron
                    ),
                );
                defs.push(def);
            }
            Err(e) => runlog::kv("weapon_error", &format!("class={class_path} error=\"{e}\"")),
        }
    }
    if defs.is_empty() {
        return;
    }
    let current = defs.len() - 1; // start with the 9mm, as KF does
    let mut w = Weapons {
        defs,
        current,
        action: Action::Select,
        sequence: None,
        frame: 0.0,
        play_rate: 30.0,
        looping: false,
        fire_cooldown: 0.0,
        fire_count: 0,
        pending_swing: None,
        aiming: false,
        zoom: 0.0,
        zoom_time: 0.25,
        rng: 0x2545_F491_4F6C_DD1D,
    };
    start_action(&mut w, Action::Select);
    commands.insert_resource(w);
    commands.insert_resource(WeaponEffects::default());
    runlog::kv("weapons_ready", &format!("seconds={:.2}", started.elapsed().as_secs_f64()));
}

fn load_weapon(
    set: &PackageSet,
    defaults: &ClassDefaults,
    class_path: &str,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<WeaponDef, String> {
    let (pkg_name, class_name) = class_path.split_once('.').ok_or("bad class path")?;
    let lp = set.load(pkg_name).ok_or("package not found")?;
    let class_export = (0..lp.pkg.exports.len())
        .find(|&i| {
            lp.pkg.export_class_name(i) == "Class"
                && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(class_name)
        })
        .ok_or("class not found")?;
    let class = ObjectHandle {
        package: lp.clone(),
        export: class_export,
    };
    let get = |p: &str| defaults.get(&class, p);

    let (mesh_value, mesh_pkg) = get("Mesh").ok_or("no Mesh default")?;
    let Value::Object(mesh_ref) = mesh_value else {
        return Err("Mesh is not an object".into());
    };
    let mesh_h = set.resolve(&mesh_pkg, mesh_ref).ok_or("mesh not found")?;

    let float = |p: &str, d: f32| match get(p) {
        Some((Value::Float(f), _)) => f,
        _ => d,
    };
    let view_offset = match get("PlayerViewOffset") {
        Some((Value::Vector(v), _)) => v,
        _ => [0.0; 3],
    };
    let display_fov = float("DisplayFOV", 90.0);
    let bob_damping = float("BobDamping", 0.96);
    let speed_bonus = if matches!(get("bSpeedMeUp"), Some((Value::Bool(true), _))) {
        // KFHumanPawn: default.GroundSpeed * BaseMeleeIncrease - Weight * 2
        200.0 * 0.2 - float("Weight", 0.0) * 2.0
    } else {
        0.0
    };

    let name = |p: &str| match get(p) {
        Some((Value::Name(n), np)) => Some(np.pkg.name(n).to_string()),
        _ => None,
    };
    let has_aiming = matches!(get("bHasAimingMode"), Some((Value::Bool(true), _)));
    let mut fire_aimed_anim = None;

    // Fire mode: animation names and rates, damage.
    let (mut fire_anims, mut fire_anim_rate, mut fire_rate) = (vec!["Fire".to_string()], 1.0, 0.5);
    let mut combat = CombatStats {
        headshot_mult: 1.0,
        ..default()
    };
    let mut ammo = None;
    if let Some((Value::Object(fm), fm_pkg)) = get("FireModeClass")
        && let Some(fm_class) = set.resolve(&fm_pkg, fm)
    {
        let fget = |p: &str| defaults.get(&fm_class, p);
        let ffloat = |p: &str, d: f32| match fget(p) {
            Some((Value::Float(f), _)) => f,
            Some((Value::Int(i), _)) => i as f32,
            _ => d,
        };
        // Melee fire modes have MeleeDamage; instant-hit ones DamageMin/Max.
        let melee_damage = ffloat("MeleeDamage", 0.0);
        combat.melee = melee_damage > 0.0;
        if combat.melee {
            combat.damage_min = melee_damage;
            combat.damage_max = melee_damage;
            combat.range = ffloat("weaponRange", 70.0);
            combat.damage_delay = ffloat("DamagedelayMin", 0.3);
            combat.min_dot = ffloat("WideDamageMinHitAngle", 1.0);
        } else {
            combat.damage_min = ffloat("DamageMin", 0.0);
            combat.damage_max = ffloat("DamageMax", 0.0);
            combat.spread = ffloat("Spread", 0.0);
        }
        // HeadShotDamageMult from the damage type class.
        let dt_name = if combat.melee { "hitDamageClass" } else { "DamageType" };
        if let Some((Value::Object(dt), dt_pkg)) = fget(dt_name)
            && let Some(dt_class) = set.resolve(&dt_pkg, dt)
            && let Some((Value::Float(m), _)) = defaults.get(&dt_class, "HeadShotDamageMult")
        {
            combat.headshot_mult = m;
        }
        // Ammo: magazine size from the weapon, starting total from the ammo class.
        if let Some((Value::Object(ac), ac_pkg)) = fget("AmmoClass")
            && let Some(ammo_class) = set.resolve(&ac_pkg, ac)
        {
            let initial = match defaults.get(&ammo_class, "InitialAmount") {
                Some((Value::Int(i), _)) => i.max(0) as u32,
                _ => 0,
            };
            let capacity = match get("MagCapacity") {
                Some((Value::Int(i), _)) => i.max(1) as u32,
                _ => 1,
            };
            let mag = capacity.min(initial);
            ammo = Some(Ammo {
                mag,
                spare: initial - mag,
                capacity,
            });
        }
        if let Some((Value::Name(n), np)) = fget("FireAnim") {
            fire_anims = vec![np.pkg.name(n).to_string()];
        }
        if let Some((Value::Name(n), np)) = fget("FireAimedAnim") {
            fire_aimed_anim = Some(np.pkg.name(n).to_string());
        }
        if let Some((Value::Array { count, raw }, np)) = fget("FireAnims") {
            let mut r = ue_assets::reader::Reader::new(&raw);
            let names: Vec<String> = (0..count)
                .filter_map(|_| r.compact_index().ok())
                .filter(|&i| i >= 0 && (i as usize) < np.pkg.names.len())
                .map(|i| np.pkg.name(i as usize).to_string())
                .collect();
            if !names.is_empty() {
                fire_anims = names;
            }
        }
        if let Some((Value::Float(f), _)) = fget("FireAnimRate") {
            fire_anim_rate = f;
        }
        if let Some((Value::Float(f), _)) = fget("FireRate") {
            fire_rate = f;
        }
    }

    // Skins override the mesh's texture slots.
    let skins: Vec<ObjectRef> = match get("Skins") {
        Some((Value::Array { count, raw }, _)) => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            (0..count)
                .filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw))
                .collect()
        }
        _ => Vec::new(),
    };
    let skins_pkg = get("Skins").map(|(_, p)| p);

    let model = SkinnedModel::load(
        set,
        &mesh_h,
        &Skins {
            refs: skins,
            package: skins_pkg,
        },
        false,
        meshes,
        images,
        materials,
    )?;
    // KFWeapon: aiming plays IdleAimAnim / FireAimedAnim (falling back to the
    // normal ones when missing, as KFFire.PlayFiring does).
    let iron = has_aiming.then(|| IronSights {
        player_fov: float("PlayerIronSightFOV", 75.0),
        display_fov: float("ZoomedDisplayFOV", display_fov),
        zoom_time: float("ZoomTime", 0.25),
        fast_zoom_out_time: float("FastZoomOutTime", 0.2),
        idle_anim: name("IdleAimAnim").unwrap_or_else(|| "Idle".into()),
        fire_anim: fire_aimed_anim.unwrap_or_else(|| fire_anims[0].clone()),
    });
    Ok(WeaponDef {
        class: class_path.to_string(),
        model,
        entities: Vec::new(),
        view_offset: coords::pos(view_offset),
        display_fov,
        iron,
        fire_anims,
        fire_anim_rate,
        fire_rate,
        speed_bonus,
        bob_damping,
        combat,
        ammo,
    })
}

/// UE2 Pawn.CalcDrawOffset scales PlayerViewOffset by 0.9 / DisplayFOV * 100.
fn draw_offset_factor(display_fov: f32) -> f32 {
    0.9 / display_fov * 100.0
}

/// Short name for logs: "KFMod.Single" -> "Single".
fn weapon_name(class: &str) -> &'static str {
    match class.rsplit('.').next() {
        Some("Knife") => "Knife",
        Some("Single") => "9mm",
        _ => "weapon",
    }
}

/// Starts the animation for an action on the current weapon.
fn start_action(w: &mut Weapons, action: Action) {
    w.action = action;
    let def = &w.defs[w.current];
    let (name, rate_scale, looping) = match action {
        Action::Select => ("select".to_string(), 1.0, false),
        Action::Idle => match &def.iron {
            Some(iron) if w.aiming => (iron.idle_anim.to_ascii_lowercase(), 1.0, true),
            _ => ("idle".to_string(), 1.0, true),
        },
        Action::Reload => ("reload".to_string(), 1.0, false),
        Action::PutDown { .. } => ("putdown".to_string(), 1.0, false),
        Action::Fire if w.aiming && def.iron.is_some() => {
            let iron = def.iron.as_ref().expect("checked");
            (iron.fire_anim.to_ascii_lowercase(), def.fire_anim_rate, false)
        }
        Action::Fire => {
            let pick = def.fire_anims[w.fire_count % def.fire_anims.len()].to_ascii_lowercase();
            (pick, def.fire_anim_rate, false)
        }
    };
    w.sequence = def.model.sequence(&name);
    w.frame = 0.0;
    w.looping = looping;
    let base_rate = w.sequence.map_or(30.0, |s| def.model.rate(s));
    w.play_rate = base_rate * rate_scale;
    runlog::kv(
        "weapon_action",
        &format!(
            "weapon={} action={action:?} sequence={name} found={} rate={:.1}",
            def.class,
            w.sequence.is_some(),
            w.play_rate
        ),
    );
}

fn grabbed_now(cursor: &Query<&bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>) -> bool {
    cursor
        .single()
        .is_ok_and(|c| c.grab_mode != bevy::window::CursorGrabMode::None)
}

/// Leaves iron sights; `fast` uses FastZoomOutTime (reload, switch).
fn zoom_out(w: &mut Weapons, fast: bool, reason: &str) {
    if !w.aiming {
        return;
    }
    let cur = w.current;
    if let Some(iron) = &w.defs[cur].iron {
        w.zoom_time = if fast { iron.fast_zoom_out_time } else { iron.zoom_time };
    }
    w.aiming = false;
    runlog::kv("iron_sights", &format!("weapon={} aiming=false reason={reason}", w.defs[cur].class));
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn weapon_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Query<&bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
    weapons: Option<ResMut<Weapons>>,
    mut effects: Option<ResMut<WeaponEffects>>,
    script: Res<ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    main_cam: Query<(&Transform, Option<&crate::walk::Walker>), With<FlyCamera>>,
    mut shots: MessageWriter<ShotFired>,
    mut swings: MessageWriter<MeleeSwing>,
    mut ammo_display: ResMut<crate::combat::AmmoDisplay>,
) {
    let Some(mut w) = weapons else {
        return;
    };
    let scripted = |action: &str| script.0.iter().any(|(f, a)| *f == frames.0 && a == action);
    w.fire_cooldown = (w.fire_cooldown - time.delta_secs()).max(0.0);
    let busy = matches!(w.action, Action::PutDown { .. } | Action::Select);
    for (key, slot, name) in [(KeyCode::Digit1, 0usize, "1"), (KeyCode::Digit2, 1usize, "2")] {
        if (keys.just_pressed(key) || scripted(name)) && slot < w.defs.len() && slot != w.current && !busy {
            zoom_out(&mut w, true, "switch");
            start_action(&mut w, Action::PutDown { next: slot });
        }
    }
    // Iron sights: right mouse toggles (KFWeapon.ToggleIronSights). Not while
    // switching, reloading, or in the air.
    if (grabbed_now(&cursor) && mouse.just_pressed(MouseButton::Right)) || scripted("aim") {
        let cur = w.current;
        let in_air = main_cam.single().is_ok_and(|(_, walker)| walker.is_some_and(|wk| !wk.on_ground));
        let ready = matches!(w.action, Action::Idle | Action::Fire);
        if w.aiming {
            zoom_out(&mut w, false, "toggle");
        } else if let Some(iron) = &w.defs[cur].iron
            && ready
            && !in_air
        {
            w.zoom_time = iron.zoom_time;
            w.aiming = true;
            runlog::kv("iron_sights", &format!("weapon={} aiming=true reason=toggle", w.defs[cur].class));
        } else {
            runlog::kv(
                "iron_sights_refused",
                &format!(
                    "weapon={} has_iron_sights={} action={:?} in_air={in_air}",
                    w.defs[cur].class,
                    w.defs[cur].iron.is_some(),
                    w.action
                ),
            );
        }
    }
    let grabbed = grabbed_now(&cursor);
    if ((grabbed && mouse.pressed(MouseButton::Left)) || scripted("fire"))
        && w.fire_cooldown <= 0.0
        && matches!(w.action, Action::Idle | Action::Fire)
    {
        let cur = w.current;
        w.fire_cooldown = w.defs[cur].fire_rate;
        let has_ammo = w.defs[cur].ammo.is_none_or(|a| a.mag > 0);
        if !has_ammo {
            runlog::kv("dry_fire", &format!("weapon={}", w.defs[cur].class));
        } else {
            if let Some(a) = w.defs[cur].ammo.as_mut() {
                a.mag -= 1;
            }
            w.fire_count += 1;
            start_action(&mut w, Action::Fire);
            let stats = w.defs[cur].combat;
            if stats.melee {
                w.pending_swing = Some(stats.damage_delay);
            } else if let Ok((cam, _)) = main_cam.single() {
                // InstantFire: random direction within the spread cone,
                // damage rolled between DamageMin and DamageMax. KFFire.GetSpread
                // halves the spread while aiming.
                let spread = if w.aiming { stats.spread * 0.5 } else { stats.spread };
                let (u, v) = (w.random() * 2.0 - 1.0, w.random() * 2.0 - 1.0);
                let dir = (*cam.forward() + *cam.right() * u * spread + *cam.up() * v * spread).normalize();
                let damage = stats.damage_min + (stats.damage_max - stats.damage_min) * w.random();
                shots.write(ShotFired {
                    origin: cam.translation,
                    dir,
                    damage,
                    headshot_mult: stats.headshot_mult,
                    weapon: weapon_name(&w.defs[cur].class),
                });
            }
        }
    }
    // Knife: damage lands DamagedelayMin seconds into the swing.
    if let Some(t) = w.pending_swing {
        let t = t - time.delta_secs();
        if t <= 0.0 {
            w.pending_swing = None;
            let stats = w.defs[w.current].combat;
            if let Ok((cam, _)) = main_cam.single() {
                swings.write(MeleeSwing {
                    origin: cam.translation,
                    dir: *cam.forward(),
                    damage: stats.damage_min,
                    range: stats.range,
                    min_dot: stats.min_dot,
                    headshot_mult: stats.headshot_mult,
                    weapon: weapon_name(&w.defs[w.current].class),
                });
            }
        } else {
            w.pending_swing = Some(t);
        }
    }
    // Reload from idle, or once the fire cooldown has passed (the fire
    // animation itself can run longer than the fire rate).
    let can_reload = w.defs[w.current].ammo.is_some_and(|a| a.mag < a.capacity && a.spare > 0);
    if (keys.just_pressed(KeyCode::KeyR) || scripted("reload"))
        && can_reload
        && (w.action == Action::Idle || (w.action == Action::Fire && w.fire_cooldown <= 0.0))
    {
        zoom_out(&mut w, true, "reload");
        start_action(&mut w, Action::Reload);
    }
    if let Some(e) = effects.as_mut() {
        e.ground_speed_bonus = w.defs[w.current].speed_bonus;
    }
    ammo_display.0 = w.defs[w.current].ammo.map(|a| (a.mag, a.spare));
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // Bevy system parameters
fn animate_weapon(
    time: Res<Time>,
    weapons: Option<ResMut<Weapons>>,
    mut meshes: ResMut<Assets<Mesh>>,
    main_cam: Query<&Transform, (With<FlyCamera>, Without<WeaponCamera>)>,
    mut weapon_cam: Query<(&mut Transform, &mut Projection), With<WeaponCamera>>,
    mut visibility: Query<&mut Visibility>,
    mut weapon_parts: Query<&mut Transform, (Without<FlyCamera>, Without<WeaponCamera>)>,
    bob: Res<crate::walk::ViewBob>,
    mut view_fov: ResMut<crate::camera::ViewFov>,
    mut log_timer: Local<f32>,
) {
    let Some(mut w) = weapons else {
        return;
    };
    let dt = time.delta_secs();

    // Iron sight zoom: blend toward the target over the zoom time. When a
    // zoom finishes while idle, switch between the hip and iron idles
    // (KFWeapon.OnZoomInFinished / OnZoomOutFinished).
    let target = if w.aiming { 1.0 } else { 0.0 };
    if w.zoom != target {
        let step = dt / w.zoom_time.max(1e-3);
        w.zoom = if w.aiming { (w.zoom + step).min(1.0) } else { (w.zoom - step).max(0.0) };
        if w.zoom == target {
            runlog::kv("iron_sights_zoom_done", &format!("aiming={}", w.aiming));
            if w.action == Action::Idle {
                start_action(&mut w, Action::Idle);
            }
        }
    }
    let def = &w.defs[w.current];
    let (display_fov, player_fov) = match &def.iron {
        Some(iron) => (
            def.display_fov + (iron.display_fov - def.display_fov) * w.zoom,
            crate::camera::DEFAULT_FOV + (iron.player_fov - crate::camera::DEFAULT_FOV) * w.zoom,
        ),
        None => (def.display_fov, crate::camera::DEFAULT_FOV),
    };
    if view_fov.0 != player_fov {
        view_fov.0 = player_fov;
    }
    // Draw offset: CalcDrawOffset at the current DisplayFOV, blended to zero
    // when aimed (a guess; the native ironsight code is not in the scripts).
    let offset = def.view_offset * draw_offset_factor(display_fov) * (1.0 - w.zoom);

    // Weapon camera follows the main camera and uses this weapon's FOV.
    if let (Ok(main), Ok((mut wt, mut proj))) = (main_cam.single(), weapon_cam.single_mut()) {
        *wt = *main;
        if let Projection::Perspective(p) = proj.as_mut() {
            p.fov = crate::camera::vertical_fov(display_fov);
        }
    }

    // Advance the animation; finished one-shots lead to the next action.
    w.frame += dt * w.play_rate;
    let length = w.sequence.map_or(1.0, |s| w.defs[w.current].model.length(s));
    if w.frame >= length {
        if w.looping {
            w.frame %= length.max(1e-3);
        } else {
            match w.action {
                Action::PutDown { next } => {
                    w.current = next;
                    start_action(&mut w, Action::Select);
                }
                Action::Reload => {
                    let cur = w.current;
                    if let Some(a) = w.defs[cur].ammo.as_mut() {
                        let n = (a.capacity - a.mag).min(a.spare);
                        a.mag += n;
                        a.spare -= n;
                        runlog::kv("reloaded", &format!("mag={} spare={}", a.mag, a.spare));
                    }
                    start_action(&mut w, Action::Idle);
                }
                _ => start_action(&mut w, Action::Idle),
            }
        }
    }

    // Weapon bob (Pawn.WeaponBob): BobDamping x WalkBob sideways, and
    // (0.45 + 0.55 x BobDamping) x WalkBob vertically. The camera already
    // moved by WalkBob, so relative to it the weapon moves by the difference.
    if let Ok(main) = main_cam.single() {
        let d = w.defs[w.current].bob_damping;
        let world = bob.side * (d - 1.0) + Vec3::Y * bob.up * (0.45 + 0.55 * d - 1.0);
        let local = main.rotation.inverse() * world;
        for &e in &w.defs[w.current].entities {
            if let Ok(mut t) = weapon_parts.get_mut(e) {
                t.translation = offset + local;
            }
        }
    }

    // Show only the current weapon.
    for (i, def) in w.defs.iter().enumerate() {
        for &e in &def.entities {
            if let Ok(mut v) = visibility.get_mut(e) {
                *v = if i == w.current { Visibility::Visible } else { Visibility::Hidden };
            }
        }
    }

    // Pose, then mesh space -> drawn: subtract MeshOrigin, scale by MeshScale (UE2).
    let def = &w.defs[w.current];
    let skinned = def.model.pose(w.sequence, w.frame);
    let scale = Vec3::from_array(def.model.mesh.scale);
    let origin = Vec3::from_array(def.model.mesh.origin);
    def.model.upload(&skinned, |p| coords::pos(((p - origin) * scale).to_array()), &mut meshes);

    *log_timer += dt;
    if *log_timer >= 1.0 {
        *log_timer = 0.0;
        let (lo, hi) = skinned.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        runlog::kv(
            "weapon_pose",
            &format!(
                "weapon={} action={:?} frame={:.1} aiming={} zoom={:.2} view_fov={:.1} display_fov={:.1} skinned_bounds_unreal=({:.1},{:.1},{:.1})..({:.1},{:.1},{:.1})",
                def.class, w.action, w.frame, w.aiming, w.zoom, view_fov.0, display_fov, lo.x, lo.y, lo.z, hi.x, hi.y, hi.z
            ),
        );
    }
}
