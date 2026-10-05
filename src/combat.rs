//! Combat: player health and HUD, shots and knife swings against zeds, zed
//! attacks against the player. Values and rules follow KF's scripts (see
//! DESIGN.md, "Combat").

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::camera::FlyCamera;
use crate::coords::SCALE;
use crate::runlog;
use crate::zed::Zed;

/// A bullet from the player's weapon (InstantFire).
#[derive(Message, Clone, Copy, Debug)]
pub struct ShotFired {
    pub origin: Vec3,
    pub dir: Vec3,
    pub damage: f32,
    pub headshot_mult: f32,
    pub weapon: &'static str,
    /// The weapon's tip (KFWeapon.GetEffectStart), Unreal world: where the
    /// tracer starts.
    pub effect_start: Option<Vec3>,
    /// Zeds one bullet can pass through: 1 for KFFire.DoTrace; 5 for the
    /// DeagleFire family, whose DoTrace halves the damage after each.
    pub max_penetrations: u32,
    /// A burning damage type (W7), if the shot's is one.
    pub fire: Option<FireType>,
}

/// A knife swing reaching its damage moment (KFMeleeFire).
#[derive(Message, Clone, Copy, Debug)]
pub struct MeleeSwing {
    pub origin: Vec3,
    pub dir: Vec3,
    pub damage: f32,
    pub range: f32,
    /// WideDamageMinHitAngle: cosine of the cone for off-centre hits;
    /// 0 = no wide hits (also the Chainsaw's held fire, whose DoFireEffect
    /// has none).
    pub min_dot: f32,
    pub headshot_mult: f32,
    pub weapon: &'static str,
}

/// Damage to the player from a zed attack.
#[derive(Message, Clone, Copy, Debug)]
pub struct PlayerDamaged {
    pub amount: f32,
    pub zed_id: usize,
    pub kind: HurtKind,
}

/// Damage types with after-effects on the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HurtKind {
    Plain,
    /// DamTypeVomit (Bloat bile): starts the bile burn.
    Vomit,
    /// DamTypeBurned (Husk fireball): sets the player on fire.
    Fire,
}

/// KFPawn fire: a burn hit of more than 2 sets BurnDown to 5 (or renews it
/// when bigger than the last) and starts a 1.5 s timer; each tick
/// (KFHumanPawn.Timer) halves LastBurnDamage and takes it as fire damage,
/// until BurnDown runs out or the damage reaches 0.
#[derive(Resource, Default)]
struct Burning {
    burn_down: u32,
    last_damage: f32,
    next: f32,
    zed_id: usize,
}

const BURN_INTERVAL: f32 = 1.5;

/// KFPawn bile: any vomit damage sets BileCount to 7; every BileFrequency
/// (0.5 s) one is used up for TakeBileDamage, 2 + Rand(3) damage.
#[derive(Resource, Default)]
struct BileBurn {
    count: u32,
    next: f32,
    zed_id: usize,
    rng: u32,
}

const BILE_FREQUENCY: f32 = 0.5;

#[derive(Resource)]
pub struct PlayerHealth {
    pub health: f32,
    pub deaths: u32,
    /// God mode (`--god` or F1): hits are still logged, but take no health.
    pub god: bool,
}

impl Default for PlayerHealth {
    fn default() -> Self {
        PlayerHealth { health: 100.0, deaths: 0, god: false }
    }
}

/// The weapon in hand and its ammo (magazine, spare), shown on the HUD
/// (written by the weapon code).
#[derive(Resource, Default)]
pub struct AmmoDisplay {
    pub weapon: &'static str,
    pub ammo: Option<(u32, u32)>,
    /// "AUTO" / "SEMI" for weapons that switch on alt fire.
    pub fire_mode: Option<&'static str>,
    /// Alt fire's own rounds (the M4 203's grenades).
    pub alt_ammo: Option<u32>,
    /// Frags left (thrown with G).
    pub frags: Option<u32>,
}

/// The player held by a Clot's grab (KFPawn.DisableMovement): no walking or
/// jumping until `seconds` runs out or the grabbing zed dies or loses its head.
#[derive(Resource, Default)]
pub struct PlayerPinned {
    pub seconds: f32,
    pub by: Option<usize>,
}

impl PlayerPinned {
    pub fn pin(&mut self, seconds: f32, zed_id: usize) {
        self.seconds = seconds;
        self.by = Some(zed_id);
        runlog::kv("player_pinned", &format!("by_zed={zed_id} seconds={seconds}"));
    }

    pub fn release(&mut self, reason: &str) {
        if self.by.is_some() {
            runlog::kv("player_released", &format!("by_zed={:?} reason={reason}", self.by));
        }
        self.seconds = 0.0;
        self.by = None;
    }

    pub fn active(&self) -> bool {
        self.seconds > 0.0
    }
}

/// KFPawn BaseEyeHeight: the eye is this far above the player's centre.
pub(crate) const PLAYER_EYE_HEIGHT: f32 = 44.0;

/// Zeds killed this session, for the HUD.
#[derive(Resource, Default)]
pub struct KillCount(pub u32);

#[derive(Component)]
struct HudText;

/// KFWeaponAttachment mTracerSpeed.
const PLAYER_TRACER_SPEED: f32 = 7500.0;

/// Total trace length for bullets (Unreal units).
const TRACE_RANGE: f32 = 10000.0;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ShotFired>()
            .add_message::<MeleeSwing>()
            .add_message::<PlayerDamaged>()
            .init_resource::<PlayerHealth>()
            .init_resource::<AmmoDisplay>()
            .init_resource::<KillCount>()
            .init_resource::<PlayerPinned>()
            .init_resource::<BileBurn>()
            .init_resource::<Burning>()
            .add_systems(Startup, spawn_hud)
            .add_systems(Update, toggle_god)
            .add_systems(Update, (resolve_shots, resolve_swings, bile_burn, fire_burn, apply_player_damage, update_hud).chain());
    }
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(28.0),
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.85, 0.75)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(20.0),
            left: Val::Px(24.0),
            ..default()
        },
        HudText,
    ));
}

/// F1 switches god mode on and off.
fn toggle_god(keys: Res<ButtonInput<KeyCode>>, mut health: ResMut<PlayerHealth>) {
    if keys.just_pressed(KeyCode::F1) {
        health.god = !health.god;
        runlog::kv("god_mode", &format!("on={}", health.god));
    }
}

fn update_hud(
    health: Res<PlayerHealth>,
    ammo: Res<AmmoDisplay>,
    kills: Res<KillCount>,
    z_spawn: Res<crate::zed::ZSpawn>,
    mut text: Query<&mut Text, With<HudText>>,
) {
    let Ok(mut t) = text.single_mut() else {
        return;
    };
    let rounds = ammo.ammo.map_or(String::new(), |(mag, spare)| format!(" {mag} / {spare}"));
    let mode = ammo.fire_mode.map_or(String::new(), |m| format!(" [{m}]"))
        + &ammo.alt_ammo.map_or(String::new(), |n| format!(" [GRENADES {n}]"));
    let frags = ammo.frags.map_or(String::new(), |n| format!("    FRAGS {n}"));
    let ammo = format!("    {}{rounds}{mode}{frags}", ammo.weapon.to_uppercase());
    **t = format!(
        "HEALTH {:.0}{}{ammo}    KILLS {}    Z: {}",
        health.health.max(0.0),
        if health.god { " (GOD)" } else { "" },
        kills.0,
        z_spawn.label.to_uppercase()
    );
}

/// Ray vs a vertical cylinder (centre, radius, half-height), Bevy space.
/// Returns the entry distance along `dir` (unit vector), if any.
pub(crate) fn ray_cylinder(origin: Vec3, dir: Vec3, centre: Vec3, radius: f32, half_height: f32) -> Option<f32> {
    let o = origin - centre;
    // Side wall: solve |(o + t d).xz| = r.
    let (a, b, c) = (
        dir.x * dir.x + dir.z * dir.z,
        2.0 * (o.x * dir.x + o.z * dir.z),
        o.x * o.x + o.z * o.z - radius * radius,
    );
    let mut best: Option<f32> = None;
    let mut consider = |t: f32| {
        if t >= 0.0 && (o.y + t * dir.y).abs() <= half_height {
            best = Some(best.map_or(t, |b: f32| b.min(t)));
        }
    };
    if a > 1e-9 {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let sq = disc.sqrt();
            consider((-b - sq) / (2.0 * a));
            consider((-b + sq) / (2.0 * a));
        }
    }
    // Caps: y = +-half_height, inside the radius.
    if dir.y.abs() > 1e-9 {
        for cap in [half_height, -half_height] {
            let t = (cap - o.y) / dir.y;
            let p = o + t * dir;
            if t >= 0.0 && p.x * p.x + p.z * p.z <= radius * radius {
                best = Some(best.map_or(t, |b: f32| b.min(t)));
            }
        }
    }
    best
}

/// Nearest entry into a zed's main or extended collision cylinder.
pub(crate) fn zed_hit(z: &Zed, origin: Vec3, dir: Vec3) -> Option<f32> {
    let main = ray_cylinder(origin, dir, z.centre, z.radius * SCALE, z.half_height * SCALE);
    let ext = z
        .ext
        .and_then(|(c, r, h)| ray_cylinder(origin, dir, c, r * SCALE, h * SCALE));
    match (main, ext) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// KFMonster.IsHeadShot: distance from the head sphere centre to the segment
/// from the hit point along the shot, length 2 x (height + radius).
pub(crate) fn is_headshot(z: &Zed, hit: Vec3, dir: Vec3, scale: f32) -> bool {
    let Some((head, _)) = z.head else {
        return false;
    };
    let seg = dir * 2.0 * (z.half_height + z.radius) * SCALE;
    let diff = head - hit;
    let t = seg.dot(diff);
    let closest = if t <= 0.0 {
        diff
    } else if t < seg.dot(seg) {
        diff - seg * (t / seg.dot(seg))
    } else {
        diff - seg
    };
    closest.length() < z.head_radius * scale * SCALE
}

/// Damage types that burn (KFWeaponDamageType.bDealBurningDamage), by the
/// rules that tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireType {
    /// DamTypeFlamethrower (also every burn tick not from the two below).
    Flamethrower,
    /// DamTypeTrenchgun (its burn ticks keep this type).
    Trenchgun,
    /// DamTypeMAC10MPInc (no x 1.5; its ticks keep this type). Firebug perk only.
    Mac10,
    /// DamTypeBurned exactly (the Bloat takes x 1.5).
    Burned,
    /// DamTypeHuskGun (a DamTypeBurned subclass).
    HuskGun,
}

/// The attacker for a hit: where it was hit, the attacker's centre, and
/// whether it was a melee attack (for the stun rule).
#[derive(Clone, Copy)]
pub(crate) struct HitSource {
    pub point: Vec3,
    pub attacker: Vec3,
    pub melee: bool,
    /// For explosive damage types: ZombieFleshPound.TakeDamage's
    /// multiplier (1 for grenades and the LAW, 2 for the frag and pipe
    /// bomb); None for everything else (x 0.5, or x 0.75 for a headshot by a
    /// damage type with HeadShotDamageMult >= 1.5).
    pub explosive: Option<f32>,
    /// A burning damage type, if this is one.
    pub fire: Option<FireType>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn damage_zed(
    z: &mut Zed,
    damage: f32,
    headshot: bool,
    headshot_mult: f32,
    weapon: &str,
    distance: f32,
    source: HitSource,
    kills: &mut KillCount,
) {
    if z.health <= 0.0 {
        return;
    }
    // Burned and flamethrower damage never gets the headshot or headless
    // multiplier (KFMonster.TakeDamage). KF still looks for a headshot with
    // DamTypeBurned, but its only hits are blasts and burn ticks, which have
    // no hit location; here they never count as headshots.
    let burned_type = matches!(source.fire, Some(FireType::Flamethrower | FireType::Burned | FireType::HuskGun));
    let headshot = headshot && !burned_type;
    // KFMonster.TakeDamage: headshots, and every hit on a headless zed, are
    // multiplied by the damage type's HeadShotDamageMult.
    let mult = if (headshot || z.decapitated) && !burned_type { headshot_mult } else { 1.0 };
    // ZombieFleshPound.TakeDamage: explosives as listed (x 1, frag and pipe
    // bomb x 2); anything else x 0.5, or x 0.75 for a headshot by a damage
    // type with HeadShotDamageMult >= 1.5.
    let fleshpound = match source.explosive {
        _ if z.small_arms_scale >= 1.0 => 1.0,
        Some(m) => m,
        None if headshot && headshot_mult >= 1.5 => 0.75,
        None => z.small_arms_scale,
    };
    // Zed overrides before KFMonster.TakeDamage: ZombieBloat x 1.5 for
    // DamTypeBurned; ZombieHusk x BurnDamageScale for DamTypeBurned and
    // DamTypeFlamethrower.
    let zed_fire = match source.fire {
        Some(FireType::Burned) => z.burned_scale * z.fire_resist,
        Some(FireType::Flamethrower) => z.fire_resist,
        _ => 1.0,
    };
    let mut damage = damage * fleshpound * zed_fire;
    // KFMonster.TakeDamage, bDealBurningDamage: remember the hit for the
    // burn ticks, x 1.5 (not the MAC10), and set the zed on fire at 15 or
    // after more than 4 lighter hits (HeatAmount).
    if let Some(f) = source.fire {
        if z.burn_down == 0 || damage > z.last_burn_damage {
            z.last_burn_damage = damage;
            z.fire_class = if matches!(f, FireType::Trenchgun | FireType::Mac10) { f } else { FireType::Flamethrower };
        }
        if f != FireType::Mac10 {
            damage *= 1.5;
        }
        if z.burn_down == 0 {
            if z.heat > 4 || damage >= 15.0 {
                z.burn_down = 10;
                z.burn_timer = 1.0;
                runlog::kv("zed_ignited", &format!("zed={} weapon={weapon} fire={f:?} damage={damage:.1} heat={}", z.id, z.heat));
            } else {
                z.heat += 1;
            }
        }
    }
    let dealt = damage * mult;
    let mut total = dealt;
    let mut head_off = false;
    let mut explosion = 0.0;
    if headshot && !z.decapitated {
        z.head_health -= dealt;
        // ZombieBoss.RemoveHead does nothing: the Patriarch keeps his head.
        if (z.head_health <= 0.0 || dealt > z.health) && !z.keeps_head {
            // RemoveHead: the head explodes for LastDamageAmount + 0.25 x
            // HealthMax more, which goes through TakeDamage again with the
            // zed headless, so it is multiplied again.
            explosion = (dealt + 0.25 * z.health_max) * headshot_mult;
            total += explosion;
            head_off = true;
            runlog::kv(
                "zed_decapitated",
                &format!("id={} weapon={weapon} hit={dealt:.1} head_explosion={explosion:.1} health_before={:.1}", z.id, z.health),
            );
        }
    }
    z.health -= total;
    z.note_damage(total);
    z.note_attacker_distance((source.attacker - z.centre).length() / SCALE);
    if head_off {
        z.remove_head();
        if z.health > 0.0 {
            runlog::kv(
                "zed_bleeding_out",
                &format!("id={} health_left={:.1} seconds={:.1}", z.id, z.health, z.bleed_out_duration),
            );
        }
    }
    // KFMonster.DoDamageFX: effects for this hit (decapitation, a severed
    // limb on the killing hit), with the health left after it.
    z.gore_hits.push(crate::zed::GoreHit {
        damage: dealt,
        health_after: z.health,
        melee: source.melee,
        decapitated: head_off,
        point: source.point,
        dir: z.last_hit.map_or(Vec3::ZERO, |(_, d)| d),
        attacker: source.attacker,
    });
    let killed = z.health <= 0.0;
    // ZombieBoss.TakeDamage: below the next healing level, knocked down.
    z.note_boss_health();
    if killed {
        z.kill();
        kills.0 += 1;
    } else {
        // Hit reactions (KFMonster.PlayHit). The head explosion is its own
        // damage event and comes first; the main hit's reaction is then
        // usually blocked by the 0.5 s pain-animation limit.
        if head_off {
            z.take_hit(explosion, source.point, source.attacker, source.melee);
        }
        z.take_hit(dealt, source.point, source.attacker, source.melee);
    }
    runlog::kv(
        "hit",
        &format!(
            "weapon={weapon} zed={} distance_unreal={:.0} damage={total:.1} headshot={headshot} health_left={:.1} head_health_left={:.1} killed={killed} decapitated={}",
            z.id,
            distance / SCALE,
            z.health.max(0.0),
            z.head_health.max(0.0),
            z.decapitated
        ),
    );
}

fn resolve_shots(
    mut shots: MessageReader<ShotFired>,
    mut bullet_fx: MessageWriter<crate::bullet_fx::BulletFx>,
    spatial: SpatialQuery,
    mut zeds: Query<&mut Zed>,
    mut kills: ResMut<KillCount>,
) {
    for shot in shots.read() {
        let max = TRACE_RANGE * SCALE;
        let world_hit = spatial.cast_ray(shot.origin, Dir3::new(shot.dir).unwrap_or(Dir3::NEG_Z), max, true, &crate::collision::world_filter());
        let world = world_hit.map_or(max, |h| h.distance);
        // Live zeds whose cylinders the ray enters before the world, nearest
        // first. DeagleFire.DoTrace: up to 5, halving the (whole-number)
        // damage after each; KFFire.DoTrace: the first only.
        let mut hits: Vec<(f32, Mut<Zed>)> = Vec::new();
        for z in &mut zeds {
            if z.health <= 0.0 {
                continue;
            }
            if let Some(t) = zed_hit(&z, shot.origin, shot.dir)
                && t < world
            {
                hits.push((t, z));
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits.truncate(shot.max_penetrations.max(1) as usize);
        let penetrating = shot.max_penetrations > 1;
        let mut hit_damage = shot.damage;
        let any_hit = !hits.is_empty();
        for (n, (t, mut z)) in hits.into_iter().enumerate() {
            {
                let hit = shot.origin + shot.dir * t;
                z.last_hit = Some((hit, shot.dir));
                let head = is_headshot(&z, hit, shot.dir, 1.0);
                if let Some((hc, _)) = z.head {
                    let to = hc - shot.origin;
                    let miss = (to - shot.dir * to.dot(shot.dir)).length() / SCALE;
                    runlog::kv(
                        "headshot_check",
                        &format!(
                            "zed={} eye_z_unreal={:.1} head_centre_z_unreal={:.1} line_to_head_centre_unreal={miss:.1} head_radius_unreal={:.1}",
                            z.id,
                            shot.origin.y / SCALE,
                            hc.y / SCALE,
                            z.head_radius
                        ),
                    );
                }
                let source = HitSource {
                    point: hit,
                    attacker: shot.origin - Vec3::Y * PLAYER_EYE_HEIGHT * SCALE,
                    melee: false,
                    explosive: None,
                    fire: shot.fire,
                };
                let damage = if penetrating { hit_damage.trunc() } else { hit_damage };
                if penetrating {
                    runlog::kv(
                        "penetration",
                        &format!("weapon={} zed={} hit_number={} damage={damage}", shot.weapon, z.id, n + 1),
                    );
                }
                damage_zed(&mut z, damage, head, shot.headshot_mult, shot.weapon, t, source, &mut kills);
                hit_damage /= 2.0;
            }
        }
        match any_hit {
            true => {}
            false => {
                runlog::kv(
                    "miss",
                    &format!("weapon={} world_hit_distance_unreal={:.0}", shot.weapon, world / SCALE),
                );
                // KFFire.DoTrace calls UpdateHit (tracer and impact) only for
                // the level and non-pawn actors. KF quirk, kept: a shot that
                // hits a zed, or hits nothing, draws neither.
                if let Some(h) = world_hit {
                    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                    let n = if h.normal.dot(shot.dir) > 0.0 { -h.normal } else { h.normal };
                    let n = to_ue(n);
                    // HitLocation + 2 x HitNormal.
                    let hit = to_ue(shot.origin + shot.dir * h.distance) / SCALE + 2.0 * n;
                    bullet_fx.write(crate::bullet_fx::BulletFx {
                        shooter: crate::bullet_fx::Shooter::Player,
                        start: shot.effect_start,
                        hit,
                        into: -n,
                        impact: true,
                        tracer_speed: PLAYER_TRACER_SPEED,
                        min_distance: 0.0,
                    });
                }
            }
        }
    }
}

/// KFMeleeFire.Timer (and ChainsawFire.DoFireEffect): a trace from the eye
/// along the view, weaponRange long, stopped by the level; the zed it hits
/// takes the damage, doubled from behind (backstab: the direction from the
/// player to the zed points the way the zed faces). Then, if
/// WideDamageMinHitAngle > 0, every other living zed in sight within
/// weaponRange x 1.1 (plus its radius) whose direction from the player is
/// within the cone takes damage x that cosine, at 0.7 of its height. The
/// wide hits use the doubled damage after a backstab (KF quirk, kept).
/// KFMonster.TakeDamage checks melee headshots with a 1.25 x larger head.
fn resolve_swings(
    mut swings: MessageReader<MeleeSwing>,
    mut zeds: Query<&mut Zed>,
    mut kills: ResMut<KillCount>,
    spatial: SpatialQuery,
) {
    for swing in swings.read() {
        let player = swing.origin - Vec3::Y * PLAYER_EYE_HEIGHT * SCALE;
        let range = swing.range * SCALE;
        let Ok(dir3) = Dir3::new(swing.dir) else { continue };
        let world_t = spatial
            .cast_ray(swing.origin, dir3, range, true, &crate::collision::world_filter())
            .map(|h| h.distance);
        let limit = world_t.unwrap_or(range);
        // The traced zed: nearest cylinder entry before the wall.
        let mut main: Option<(f32, usize)> = None;
        for z in &zeds {
            if z.health <= 0.0 {
                continue;
            }
            if let Some(t) = zed_hit(z, swing.origin, swing.dir)
                && t <= limit
                && main.is_none_or(|(bt, _)| t < bt)
            {
                main = Some((t, z.id));
            }
        }
        let mut my_damage = swing.damage;
        if let Some((t, id)) = main
            && let Some(mut z) = zeds.iter_mut().find(|z| z.id == id)
        {
            let to = (z.centre - player).normalize_or_zero();
            let backstab = to.dot(crate::zed::dir_of(z.yaw)) > 0.0;
            if backstab {
                my_damage *= 2.0;
            }
            let hit = swing.origin + swing.dir * t;
            z.last_hit = Some((hit, swing.dir));
            let head = is_headshot(&z, hit, swing.dir, 1.25);
            runlog::kv(
                "melee_hit",
                &format!("weapon={} zed={} kind=traced backstab={backstab} damage={my_damage:.1} headshot={head}", swing.weapon, z.id),
            );
            let source = HitSource { point: hit, attacker: player, melee: true, explosive: None, fire: None };
            damage_zed(&mut z, my_damage, head, swing.headshot_mult, swing.weapon, t, source, &mut kills);
        } else if let Some(t) = world_t {
            runlog::kv("melee_hit_world", &format!("weapon={} distance_unreal={:.0}", swing.weapon, t / SCALE));
        }
        let mut wide_hits = 0;
        if swing.min_dot > 0.0 {
            for mut z in &mut zeds {
                if z.health <= 0.0 || main.is_some_and(|(_, id)| id == z.id) {
                    continue;
                }
                let d = z.centre - player;
                let reach = swing.range * 1.1 * SCALE;
                if d.length_squared() > reach * reach + (z.radius * SCALE).powi(2) {
                    continue;
                }
                // VisibleCollidingActors: in sight from the eye.
                let to_eye = z.centre - swing.origin;
                if let Ok(d3) = Dir3::new(to_eye)
                    && spatial
                        .cast_ray(swing.origin, d3, to_eye.length(), true, &crate::collision::world_filter())
                        .is_some()
                {
                    continue;
                }
                let diff = swing.dir.dot(d.normalize_or_zero());
                if diff <= swing.min_dot {
                    continue;
                }
                let point = z.centre + Vec3::Y * z.half_height * 0.7 * SCALE;
                z.last_hit = Some((point, swing.dir));
                let head = is_headshot(&z, point, swing.dir, 1.25);
                let damage = my_damage * diff;
                wide_hits += 1;
                runlog::kv(
                    "melee_hit",
                    &format!("weapon={} zed={} kind=wide angle_cos={diff:.2} damage={damage:.1} headshot={head}", swing.weapon, z.id),
                );
                let source = HitSource { point, attacker: player, melee: true, explosive: None, fire: None };
                damage_zed(&mut z, damage, head, swing.headshot_mult, swing.weapon, d.length(), source, &mut kills);
            }
        }
        if main.is_none() && wide_hits == 0 {
            runlog::kv("miss", &format!("weapon={} melee=true", swing.weapon));
        }
    }
}

/// KFPawn.Tick: the bile burn's next tick.
fn bile_burn(time: Res<Time>, mut bile: ResMut<BileBurn>, mut out: MessageWriter<PlayerDamaged>) {
    if bile.count == 0 {
        return;
    }
    let now = time.elapsed_secs();
    if bile.next < now {
        bile.count -= 1;
        bile.next += BILE_FREQUENCY;
        bile.rng = bile.rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let amount = 2.0 + ((bile.rng >> 16) % 3) as f32;
        runlog::kv("player_bile", &format!("damage={amount} left={}", bile.count));
        out.write(PlayerDamaged {
            amount,
            zed_id: bile.zed_id,
            kind: crate::combat::HurtKind::Plain,
        });
    }
}

/// KFHumanPawn.Timer while burning.
fn fire_burn(time: Res<Time>, mut burn: ResMut<Burning>, mut out: MessageWriter<PlayerDamaged>) {
    if burn.burn_down == 0 || time.elapsed_secs() < burn.next {
        return;
    }
    burn.next += BURN_INTERVAL;
    // LastBurnDamage *= 0.5 (an int); TakeFireDamage: 0 puts the fire out.
    burn.last_damage = (burn.last_damage * 0.5).floor();
    if burn.last_damage <= 0.0 {
        burn.burn_down = 0;
        runlog::kv("player_burn", "out=true");
        return;
    }
    burn.burn_down -= 1;
    runlog::kv("player_burn", &format!("damage={} left={}", burn.last_damage, burn.burn_down));
    out.write(PlayerDamaged {
        amount: burn.last_damage,
        zed_id: burn.zed_id,
        kind: HurtKind::Plain,
    });
}

#[allow(clippy::too_many_arguments)]
fn apply_player_damage(
    mut hits: MessageReader<PlayerDamaged>,
    mut health: ResMut<PlayerHealth>,
    mut player: Query<(&mut Transform, Option<&mut crate::walk::Walker>), With<FlyCamera>>,
    spawn: Res<crate::map::SpawnPoint>,
    mut pinned: ResMut<PlayerPinned>,
    mut bile: ResMut<BileBurn>,
    mut burn: ResMut<Burning>,
    time: Res<Time>,
) {
    for hit in hits.read() {
        if !health.god {
            health.health -= hit.amount;
        }
        runlog::kv(
            "player_hit",
            &format!(
                "zed={} damage={} kind={:?} health_left={:.0} god={}",
                if hit.zed_id == usize::MAX { "self".to_string() } else { hit.zed_id.to_string() },
                hit.amount,
                hit.kind,
                health.health.max(0.0),
                health.god
            ),
        );
        // KFPawn.TakeDamage: DamTypeVomit -> BileCount 7.
        // KFPawn.TakeDamage: DamTypeBurned over 2 sets the player on fire.
        if hit.kind == HurtKind::Fire && hit.amount > 2.0 {
            if burn.burn_down > 0 && hit.amount > burn.last_damage {
                burn.burn_down = 5;
            }
            burn.last_damage = hit.amount;
            if burn.burn_down == 0 {
                burn.burn_down = 5;
                burn.next = time.elapsed_secs() + BURN_INTERVAL;
            }
            burn.zed_id = hit.zed_id;
        }
        if hit.kind == HurtKind::Vomit {
            let now = time.elapsed_secs();
            bile.count = 7;
            bile.zed_id = hit.zed_id;
            bile.rng ^= (now * 1000.0) as u32 | 1;
            if bile.next < now {
                bile.next = now + BILE_FREQUENCY;
            }
        }
        if health.health <= 0.0 {
            bile.count = 0;
            burn.burn_down = 0;
            // Death: respawn at the player start with full health.
            health.deaths += 1;
            health.health = 100.0;
            pinned.release("player_died");
            if let Ok((mut t, walker)) = player.single_mut() {
                t.translation = spawn.position;
                if let Some(mut w) = walker {
                    w.center = spawn.position - Vec3::Y * 44.0 * SCALE;
                    w.velocity = Vec3::ZERO;
                    w.time = 0.0;
                }
            }
            runlog::kv("player_died", &format!("deaths={} respawned=true", health.deaths));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hit from straight in front of a zed at the origin facing +X (Unreal),
    /// i.e. Bevy -Z.
    const SRC: HitSource = HitSource {
        point: Vec3::new(0.0, 0.0, -0.5),
        attacker: Vec3::new(0.0, 0.0, -4.0),
        melee: false,
        explosive: None,
        fire: None,
    };

    // The test Clot stands at the origin facing Unreal +X (Bevy -Z); Unreal
    // +Y (its right) is Bevy +X.
    fn reaction(damage: f32, point: Vec3, attacker: Vec3, melee: bool) -> Option<crate::zed::HitReaction> {
        Zed::test_clot().take_hit(damage, point, attacker, melee)
    }

    #[test]
    fn hit_reactions_follow_kf_rules() {
        use crate::zed::HitReaction::*;
        let front = Vec3::new(0.0, 0.0, -0.5);
        let far = Vec3::new(0.0, 0.0, -4.0);
        assert_eq!(reaction(30.0, front, far, false), Some(Front));
        assert_eq!(reaction(30.0, Vec3::new(0.0, 0.0, 0.5), far, false), Some(Back));
        assert_eq!(reaction(30.0, Vec3::new(0.5, 0.0, 0.0), far, false), Some(Right));
        assert_eq!(reaction(30.0, Vec3::new(-0.5, 0.0, 0.0), far, false), Some(Left));
        // Half the default health (65) or more from the front: stun.
        assert_eq!(reaction(70.0, front, far, false), Some(Stun));
        // More than Health / 1.5 (86.7): knock-down.
        assert_eq!(reaction(90.0, front, far, false), Some(KnockDown));
        // Under 5 damage: no animation.
        assert_eq!(reaction(4.0, front, far, false), None);
        // Close melee (attacker within 2 x MeleeRange = 40 units) over 13: stun.
        assert_eq!(reaction(19.0, front, Vec3::new(0.0, 0.0, -0.6), true), Some(Stun));
        // Same knife hit from 46 units (the closest the Clot and player
        // cylinders allow): ordinary front flinch.
        assert_eq!(reaction(19.0, front, Vec3::new(0.0, 0.0, -0.92), true), Some(Front));
    }

    #[test]
    fn pain_animations_at_most_every_half_second_and_stun_blocks() {
        let mut z = Zed::test_clot();
        let front = Vec3::new(0.0, 0.0, -0.5);
        let far = Vec3::new(0.0, 0.0, -4.0);
        assert!(z.take_hit(70.0, front, far, false).is_some());
        assert!(z.take_hit(30.0, front, far, false).is_none());
    }

    #[test]
    fn patriarch_keeps_head_and_never_flinches() {
        // A headshot well over his head health: no decapitation and no head
        // explosion damage, just the hit (x 1.1).
        let mut z = Zed::test_patriarch();
        let mut kills = KillCount::default();
        damage_zed(&mut z, 200.0, true, 1.1, "9mm", 1.0, SRC, &mut kills);
        assert!(!z.decapitated);
        assert!(z.head_health <= 0.0);
        assert!((z.health - 3780.0).abs() < 0.01, "{}", z.health);
        // A second headshot still does not take the head.
        damage_zed(&mut z, 200.0, true, 1.1, "9mm", 1.0, SRC, &mut kills);
        assert!(!z.decapitated);
        // Big hits from the front, close melee: no flinch, stun or knockdown.
        assert_eq!(z.take_hit(3000.0, SRC.point, SRC.attacker, true), None);
    }

    #[test]
    fn headshot_decapitates_and_clot_bleeds_out() {
        // 9mm headshot: 27.3 x 1.1 = 30 > head health 25. Head explosion
        // (30 + 0.25 x 130) x 1.1 = 68.75. 130 - 30 - 68.75 = 31.25 left.
        let mut z = Zed::test_clot();
        let mut kills = KillCount::default();
        damage_zed(&mut z, 30.0 / 1.1, true, 1.1, "9mm", 1.0, SRC, &mut kills);
        assert!(z.decapitated);
        assert!((z.health - 31.25).abs() < 0.01, "{}", z.health);
        assert_eq!(z.bleed_out, Some(5.0));
        assert!(!z.is_dead());
        assert_eq!(kills.0, 0);
        // Headless: a body shot now counts as a headshot (x1.1): 25 x 1.1 = 27.5.
        damage_zed(&mut z, 25.0, false, 1.1, "9mm", 1.0, SRC, &mut kills);
        assert!((z.health - 3.75).abs() < 0.01, "{}", z.health);
    }

    #[test]
    fn head_explosion_can_kill_outright() {
        // A hurt Clot (40 health): the head explosion alone finishes it.
        let mut z = Zed::test_clot();
        z.health = 40.0;
        let mut kills = KillCount::default();
        damage_zed(&mut z, 30.0 / 1.1, true, 1.1, "9mm", 1.0, SRC, &mut kills);
        assert!(z.is_dead() && z.decapitated);
        assert_eq!(z.bleed_out, None);
        assert_eq!(kills.0, 1);
    }

    fn fire_src(f: FireType) -> HitSource {
        HitSource { fire: Some(f), ..SRC }
    }

    #[test]
    fn fire_ignites_at_15_or_after_five_light_hits() {
        // KFMonster.TakeDamage: x 1.5; 10 x 1.5 = 15 lights at once.
        let mut z = Zed::test_patriarch();
        let mut kills = KillCount::default();
        damage_zed(&mut z, 10.0, false, 1.0, "t", 1.0, fire_src(FireType::Trenchgun), &mut kills);
        assert_eq!(z.burn_down, 10);
        assert_eq!(z.last_burn_damage, 10.0);
        assert_eq!(z.fire_class, FireType::Trenchgun);
        // 5 x 1.5 = 7.5: HeatAmount goes up; the 6th light hit lights it.
        let mut z = Zed::test_patriarch();
        for i in 0..5 {
            damage_zed(&mut z, 5.0, false, 1.0, "t", 1.0, fire_src(FireType::Flamethrower), &mut kills);
            assert_eq!(z.burn_down, 0, "hit {i}");
        }
        assert_eq!(z.heat, 5);
        damage_zed(&mut z, 5.0, false, 1.0, "t", 1.0, fire_src(FireType::Flamethrower), &mut kills);
        assert_eq!(z.burn_down, 10);
        // A weaker hit while burning keeps the stronger LastBurnDamage.
        damage_zed(&mut z, 2.0, false, 1.0, "t", 1.0, fire_src(FireType::Flamethrower), &mut kills);
        assert_eq!(z.last_burn_damage, 5.0);
    }

    #[test]
    fn fire_types_skip_the_headshot_multiplier_and_husk_resists() {
        let mut kills = KillCount::default();
        // DamTypeBurned headshot: 10 x 1.5, no x 2.
        let mut z = Zed::test_patriarch();
        let h = z.health;
        damage_zed(&mut z, 10.0, true, 2.0, "t", 1.0, fire_src(FireType::Burned), &mut kills);
        assert!((h - z.health - 15.0).abs() < 0.01, "{}", h - z.health);
        // The Trenchgun's type does get it: 10 x 1.5 x 2.
        let mut z = Zed::test_patriarch();
        damage_zed(&mut z, 10.0, true, 2.0, "t", 1.0, fire_src(FireType::Trenchgun), &mut kills);
        assert!((h - z.health - 30.0).abs() < 0.01, "{}", h - z.health);
        // A Husk (BurnDamageScale 0.25): flamethrower 20 x 0.25 x 1.5.
        let mut z = Zed::test_patriarch();
        z.fire_resist = 0.25;
        damage_zed(&mut z, 20.0, false, 1.0, "t", 1.0, fire_src(FireType::Flamethrower), &mut kills);
        assert!((h - z.health - 7.5).abs() < 0.01, "{}", h - z.health);
        // A Bloat takes x 1.5 for exactly DamTypeBurned, not the Husk Gun's.
        let mut z = Zed::test_patriarch();
        z.burned_scale = 1.5;
        damage_zed(&mut z, 10.0, false, 1.0, "t", 1.0, fire_src(FireType::Burned), &mut kills);
        assert!((h - z.health - 22.5).abs() < 0.01, "{}", h - z.health);
        let mut z = Zed::test_patriarch();
        z.burned_scale = 1.5;
        damage_zed(&mut z, 10.0, false, 1.0, "t", 1.0, fire_src(FireType::HuskGun), &mut kills);
        assert!((h - z.health - 15.0).abs() < 0.01, "{}", h - z.health);
    }

    #[test]
    fn headless_clot_claws_at_double_reach() {
        let mut z = Zed::test_clot();
        assert_eq!(z.melee_values(), (20.0, 6.0));
        let mut kills = KillCount::default();
        damage_zed(&mut z, 30.0 / 1.1, true, 1.1, "9mm", 1.0, SRC, &mut kills);
        assert!(z.decapitated);
        // ZombieClot.RemoveHead: MeleeRange x 2, MeleeDamage x 2.
        assert_eq!(z.melee_values(), (40.0, 12.0));
    }

    #[test]
    fn knife_needs_two_headshots() {
        // Knife 19 x 1.25 = 23.75 < 25 head health: the first stab leaves the head on.
        let mut z = Zed::test_clot();
        let mut kills = KillCount::default();
        damage_zed(&mut z, 19.0, true, 1.25, "Knife", 1.0, SRC, &mut kills);
        assert!(!z.decapitated);
        damage_zed(&mut z, 19.0, true, 1.25, "Knife", 1.0, SRC, &mut kills);
        assert!(z.decapitated);
    }

    #[test]
    fn ray_hits_cylinder_side() {
        let t = ray_cylinder(Vec3::new(-5.0, 0.0, 0.0), Vec3::X, Vec3::ZERO, 1.0, 2.0).unwrap();
        assert!((t - 4.0).abs() < 1e-5);
    }

    #[test]
    fn ray_misses_above_cylinder() {
        assert!(ray_cylinder(Vec3::new(-5.0, 3.0, 0.0), Vec3::X, Vec3::ZERO, 1.0, 2.0).is_none());
    }

    #[test]
    fn ray_hits_cap_from_above() {
        let t = ray_cylinder(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, Vec3::ZERO, 1.0, 2.0).unwrap();
        assert!((t - 8.0).abs() < 1e-5);
    }
}
