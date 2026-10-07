//! The end of the match: "Your squad survived" / "Your squad was wiped
//! out". KFGameType.CheckEndGame, the MatchOver state (DeathMatch,
//! KFGameType), PlayerController's GameEnded state and
//! HUDKillingFloor.DrawEndGameHUD / Tick. The wave game (waves.rs) decides
//! won or lost; this follows it: the picture and its fade, the frozen
//! player, the zeds stopping, and the restart (Fire after 5 s, or by
//! itself about 14 s after the end). See DESIGN.md, "End of match screen".

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::{Value, read_export_properties};

use crate::engine::runlog;
use crate::game::waves::{Phase, WaveGame};
use crate::world::map::MapRequest;

/// UnrealMPGameInfo.EndTimeDelay (KillingFloor.ini): EndTime = end + 4.
const END_TIME_DELAY: f32 = 4.0;
/// DeathMatch.RestartWait (KillingFloor.ini): MatchOver.Timer restarts
/// the game once Level.TimeSeconds > EndTime + RestartWait.
const RESTART_WAIT: f32 = 10.0;
/// GameEnded.BeginState: bFrozen, SetTimer(5): Fire does nothing before.
const FROZEN_SECONDS: f32 = 5.0;
/// HUDKillingFloor.Tick: EndGameHUDTime += deltaTime / 3 while under 1.
const FADE_SECONDS: f32 = 3.0;
/// DrawEndGameHUD: Scalar = FClamp(C.ClipY, 320, 1024).
const MIN_SIDE: f32 = 320.0;
const MAX_SIDE: f32 = 1024.0;
/// Above the HUD's image pool (hud.rs: 10 ..= 105) when won; below it when
/// lost (the circle and messages are drawn after the picture then).
const Z_ABOVE_HUD: i32 = 200;
const Z_BELOW_HUD: i32 = 9;

/// The match state the rest of the game looks at (GRI.EndGameType and the
/// player's GameEnded state).
#[derive(Resource, Default, Debug)]
pub struct MatchOver {
    /// Game seconds when the match ended (None: still playing).
    ended_at: Option<f32>,
    /// EndGameType 2 (won) or 1 (lost).
    pub victory: bool,
    /// HUDKillingFloor.EndGameHUDTime: the picture's fade, 0 to 1.
    pub hud_time: f32,
    restart_sent: bool,
}

impl MatchOver {
    /// GRI.EndGameType > 0: the player is in GameEnded (no moving or
    /// firing), the HUD is the spectating HUD.
    pub fn active(&self) -> bool {
        self.ended_at.is_some()
    }
}

/// Asks the wave game to start over (GameInfo.RestartGame).
#[derive(Message, Clone, Copy, Debug)]
pub struct RestartGame;

pub struct EndGamePlugin;

impl Plugin for EndGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MatchOver>()
            .init_resource::<EndPictures>()
            .add_message::<RestartGame>()
            .add_plugins(UiMaterialPlugin::<EndGameMaterial>::default())
            .add_systems(PostStartup, load_pictures)
            .add_systems(Update, (test_kill_player, follow_match.after(crate::game::waves::wave_timer)))
            .add_systems(PostUpdate, draw_picture);
        app.world_mut()
            .resource_mut::<Assets<bevy::shader::Shader>>()
            .insert(&END_GAME_SHADER, bevy::shader::Shader::from_wgsl(END_GAME_WGSL, "end_game.rs/end_game.wgsl"))
            .expect("shader handle");
    }
}

/// One of KFMapEndTextures' combiners: Material1 (the picture) added to
/// Material2, a TexOscillator (OT_Pan) over the glow texture.
#[derive(Clone, Debug, Default)]
struct Picture {
    picture: Handle<Image>,
    overlay: Handle<Image>,
    /// UOscillationRate, VOscillationRate (per second).
    rate: Vec2,
    /// UOscillationAmplitude, VOscillationAmplitude (texture widths).
    amplitude: Vec2,
}

#[derive(Resource, Default)]
struct EndPictures {
    victory: Option<Picture>,
    defeat: Option<Picture>,
}

fn float(props: &ue_assets::properties::PropertyList, h: &ObjectHandle, name: &str) -> f32 {
    match props.get(&h.package.pkg, name) {
        Some(Value::Float(f)) => *f,
        _ => 0.0,
    }
}

/// Reads a combiner and its oscillator; the reason when it cannot.
fn read_picture(set: &PackageSet, path: &str, images: &mut Assets<Image>) -> Result<Picture, String> {
    let combiner = set.find_object(path, Some("Combiner")).ok_or("no combiner")?;
    let props = read_export_properties(&combiner.package.pkg, combiner.export).map_err(|e| e.to_string())?;
    let follow = |props: &ue_assets::properties::PropertyList, h: &ObjectHandle, name: &str| match props.get(&h.package.pkg, name) {
        Some(Value::Object(rf)) => set.resolve(&h.package, *rf),
        _ => None,
    };
    // CombineOperation 3 = CO_Add (Combiner.uc's EColorOperation).
    let op = match props.get(&combiner.package.pkg, "CombineOperation") {
        Some(Value::Byte(b)) => *b,
        _ => 0,
    };
    if op != 3 {
        return Err(format!("CombineOperation={op}, expected 3 (CO_Add)"));
    }
    let picture_tex = follow(&props, &combiner, "Material1").ok_or("no Material1")?;
    let osc = follow(&props, &combiner, "Material2").ok_or("no Material2")?;
    if !osc.class_name().eq_ignore_ascii_case("TexOscillator") {
        return Err(format!("Material2 is a {}", osc.class_name()));
    }
    let osc_props = read_export_properties(&osc.package.pkg, osc.export).map_err(|e| e.to_string())?;
    let overlay_tex = follow(&osc_props, &osc, "Material").ok_or("no oscillator Material")?;
    let picture = crate::render::particles::decode(&picture_tex, false, false, images).ok_or("picture not decoded")?;
    let overlay = crate::render::particles::decode(&overlay_tex, false, false, images).ok_or("overlay not decoded")?;
    Ok(Picture {
        picture,
        overlay,
        rate: Vec2::new(float(&osc_props, &osc, "UOscillationRate"), float(&osc_props, &osc, "VOscillationRate")),
        amplitude: Vec2::new(float(&osc_props, &osc, "UOscillationAmplitude"), float(&osc_props, &osc, "VOscillationAmplitude")),
    })
}

fn load_pictures(mut pictures: ResMut<EndPictures>, request: Res<MapRequest>, mut images: ResMut<Assets<Image>>) {
    let set = PackageSet::new(&request.install_root);
    let mut report = Vec::new();
    for (path, victory) in [("KFMapEndTextures.VictoryCombiner", true), ("KFMapEndTextures.DefeatCombiner", false)] {
        match read_picture(&set, path, &mut images) {
            Ok(p) => {
                report.push(format!("{path}=(rate={:?} amplitude={:?})", p.rate.to_array(), p.amplitude.to_array()));
                if victory {
                    pictures.victory = Some(p);
                } else {
                    pictures.defeat = Some(p);
                }
            }
            Err(e) => report.push(format!("{path}=missing({e})")),
        }
    }
    runlog::kv("end_game_loaded", &report.join(" "));
}

/// Follows the wave game: the end (CheckEndGame), EndGameHUDTime, the
/// restart (Fire after 5 s, or EndTime + RestartWait), and back to play
/// after a restart (the player revived at the start).
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn follow_match(
    time: Res<Time>,
    game: Res<WaveGame>,
    mut over: ResMut<MatchOver>,
    mut restart: MessageWriter<RestartGame>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Query<&bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    (mut health, mut armour, spawn): (ResMut<crate::game::combat::PlayerHealth>, ResMut<crate::player::armour::Armour>, Res<crate::world::map::SpawnPoint>),
    mut player: Query<(&mut Transform, Option<&mut crate::player::walk::Walker>), With<crate::engine::camera::FlyCamera>>,
    mut view: ResMut<crate::engine::view_target::ViewTarget>,
    mut respawned: MessageWriter<crate::game::perks::RespawnPawn>,
) {
    let now = time.elapsed_secs();
    let ended = matches!(game.phase, Phase::Won | Phase::Lost);
    match (ended, over.ended_at) {
        (true, None) => {
            *over = MatchOver { ended_at: Some(now), victory: game.phase == Phase::Won, hud_time: 0.0, restart_sent: false };
            // ClientSetBehindView(true): no first-person weapon.
            view.set_behind_self(true);
            runlog::kv(
                "end_game",
                &format!(
                    "result={} wave={} at={now:.2} fire_restarts_from={:.2} restart_at={:.2} player_dead={}",
                    if over.victory { "survived" } else { "wiped_out" },
                    game.wave_num + 1,
                    now + FROZEN_SECONDS,
                    now + END_TIME_DELAY + RESTART_WAIT,
                    health.dead
                ),
            );
        }
        (false, Some(_)) => {
            *over = MatchOver::default();
            view.set_behind_self(false);
            // RestartGame reloads the map in KF: a new pawn at a player
            // start, full health, no armour.
            if health.dead {
                health.dead = false;
                health.health = 100.0;
                health.to_give = 0.0;
                *armour = crate::player::armour::Armour::default();
                // The starting inventory again (the dead pawn dropped its
                // weapon in hand: drop.rs).
                respawned.write(crate::game::perks::RespawnPawn);
                if let Ok((mut t, walker)) = player.single_mut() {
                    t.translation = spawn.position;
                    if let Some(mut w) = walker {
                        w.center = spawn.position - Vec3::Y * 44.0 * crate::engine::coords::SCALE;
                        w.velocity = Vec3::ZERO;
                        w.time = 0.0;
                    }
                }
            }
            runlog::kv("end_game_over", &format!("at={now:.2} health={:.0}", health.health));
        }
        _ => {}
    }
    let Some(end) = over.ended_at else { return };
    if over.hud_time < 1.0 {
        over.hud_time += time.delta_secs() / FADE_SECONDS;
        if over.hud_time >= 1.0 {
            runlog::kv("end_game_fade", &format!("hud_time={:.3} seconds={:.2}", over.hud_time, now - end));
        }
    }
    if over.restart_sent {
        return;
    }
    let grabbed = cursor.single().is_ok_and(|c| c.grab_mode != bevy::window::CursorGrabMode::None);
    let fire = (grabbed && mouse.just_pressed(MouseButton::Left)) || script.0.iter().any(|(f, a)| *f == frames.0 && a == "fire");
    let why = if now > end + END_TIME_DELAY + RESTART_WAIT {
        Some("timer")
    } else if fire && now >= end + FROZEN_SECONDS {
        Some("fire")
    } else {
        None
    };
    if let Some(why) = why {
        over.restart_sent = true;
        restart.write(RestartGame);
        runlog::kv("end_game_restart", &format!("reason={why} seconds_after_end={:.2}", now - end));
    }
}

/// Test actions "kill_player": 1000 damage from the level (not in god
/// mode); "hurt_player": 60 (for healing tests).
fn test_kill_player(
    script: Res<crate::weapons::weapon::ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut hurt: MessageWriter<crate::game::combat::PlayerDamaged>,
) {
    for (_, a) in script.0.iter().filter(|(f, a)| *f == frames.0 && (a == "kill_player" || a == "hurt_player")) {
        hurt.write(crate::game::combat::PlayerDamaged {
            amount: if a == "kill_player" { 1000.0 } else { 60.0 },
            zed_id: crate::game::combat::LEVEL_DAMAGE,
            kind: crate::game::combat::HurtKind::Plain,
            armor_stops: false,
            dam_type: crate::game::combat::DamType::Other,
            source: None,
            dam: None,
            to_peer: None,
        });
        runlog::kv("test_kill_player", &format!("frame={} action={a}", frames.0));
    }
}

/// DrawEndGameHUD's tile: the combiner as a UI material.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct EndGameMaterial {
    /// xy: the oscillator's UV offset; z: the fade (EndGameHUDTime).
    #[uniform(0)]
    params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    picture: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    overlay: Handle<Image>,
}

const END_GAME_SHADER: Handle<bevy::shader::Shader> = bevy::asset::uuid_handle!("3f6b2d8e-5a1c-4e9f-b7d2-8c4a6e1f0b93");

/// Colour: picture + overlay (CO_Add). Alpha: picture + overlay
/// (AO_Use_Mask with no mask is native: a guess), times the fade.
const END_GAME_WGSL: &str = r#"
#import bevy_ui::ui_vertex_output::UiVertexOutput

@group(1) @binding(0) var<uniform> params: vec4<f32>;
@group(1) @binding(1) var picture: texture_2d<f32>;
@group(1) @binding(2) var picture_sampler: sampler;
@group(1) @binding(3) var overlay: texture_2d<f32>;
@group(1) @binding(4) var overlay_sampler: sampler;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let a = textureSample(picture, picture_sampler, in.uv);
    let b = textureSample(overlay, overlay_sampler, in.uv + params.xy);
    return vec4<f32>(min(a.rgb + b.rgb, vec3<f32>(1.0)), min(a.a + b.a, 1.0) * params.z);
}
"#;

impl UiMaterial for EndGameMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        END_GAME_SHADER.into()
    }
}

#[derive(Component)]
struct EndPictureNode;

/// TexOscillator (OT_Pan, native): offset = Amplitude x sin(2 pi x Rate x
/// t). The sine fits the matrix stored in the package (ZPlane = 0.01 x
/// 0.994 etc.); the 2 pi and the time base are assumed.
fn oscillator_offset(p: &Picture, t: f32) -> Vec2 {
    let tau = std::f32::consts::TAU;
    Vec2::new(p.amplitude.x * (tau * p.rate.x * t).sin(), p.amplitude.y * (tau * p.rate.y * t).sin())
}

/// DrawEndGameHUD: a white square of side Clamp(ClipY, 320, 1024) canvas
/// pixels, centred, alpha EndGameHUDTime x 255.
fn picture_rect(window_physical: Vec2, scale_factor: f32) -> Rect {
    let side = window_physical.y.clamp(MIN_SIDE, MAX_SIDE);
    let min = (window_physical / 2.0 - Vec2::splat(side / 2.0)) / scale_factor;
    Rect::from_corners(min, min + Vec2::splat(side / scale_factor))
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn draw_picture(
    mut commands: Commands,
    over: Res<MatchOver>,
    pictures: Res<EndPictures>,
    time: Res<Time>,
    window: Query<&Window>,
    mut materials: ResMut<Assets<EndGameMaterial>>,
    mut node: Query<(&mut Node, &MaterialNode<EndGameMaterial>, &mut Visibility, &mut GlobalZIndex), With<EndPictureNode>>,
    mut logged: Local<bool>,
) {
    let picture = if over.victory { pictures.victory.as_ref() } else { pictures.defeat.as_ref() };
    let Ok((mut n, handle, mut vis, mut z)) = node.single_mut() else {
        let Some(p) = pictures.victory.as_ref().or(pictures.defeat.as_ref()) else { return };
        let material = materials.add(EndGameMaterial { params: Vec4::ZERO, picture: p.picture.clone(), overlay: p.overlay.clone() });
        commands.spawn((
            Node { position_type: PositionType::Absolute, ..default() },
            MaterialNode(material),
            Visibility::Hidden,
            GlobalZIndex(Z_ABOVE_HUD),
            EndPictureNode,
        ));
        return;
    };
    let (true, Some(p), Ok(win)) = (over.active(), picture, window.single()) else {
        *vis = Visibility::Hidden;
        *logged = false;
        return;
    };
    let rect = picture_rect(Vec2::new(win.physical_width() as f32, win.physical_height() as f32), win.scale_factor());
    n.left = Val::Px(rect.min.x);
    n.top = Val::Px(rect.min.y);
    n.width = Val::Px(rect.width());
    n.height = Val::Px(rect.height());
    *z = GlobalZIndex(if over.victory { Z_ABOVE_HUD } else { Z_BELOW_HUD });
    *vis = Visibility::Inherited;
    if let Some(mut m) = materials.get_mut(&handle.0) {
        let offset = oscillator_offset(p, time.elapsed_secs());
        m.params = Vec4::new(offset.x, offset.y, over.hud_time.min(1.0), 0.0);
        if m.picture != p.picture {
            m.picture = p.picture.clone();
            m.overlay = p.overlay.clone();
        }
    }
    if !*logged {
        *logged = true;
        runlog::kv(
            "end_game_picture",
            &format!(
                "which={} rect=({:.0},{:.0})-({:.0},{:.0}) window={}x{} scale={}",
                if over.victory { "VictoryCombiner" } else { "DefeatCombiner" },
                rect.min.x,
                rect.min.y,
                rect.max.x,
                rect.max.y,
                win.physical_width(),
                win.physical_height(),
                win.scale_factor()
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_is_a_centred_square_clamped_to_320_1024() {
        // 1920 x 1080: side 1024, centred.
        let r = picture_rect(Vec2::new(1920.0, 1080.0), 1.0);
        assert_eq!((r.min, r.max), (Vec2::new(448.0, 28.0), Vec2::new(1472.0, 1052.0)));
        // 1280 x 960: side 960.
        let r = picture_rect(Vec2::new(1280.0, 960.0), 1.0);
        assert_eq!((r.min, r.max), (Vec2::new(160.0, 0.0), Vec2::new(1120.0, 960.0)));
        // Tiny window: 320, overflowing; HiDPI: logical pixels.
        assert_eq!(picture_rect(Vec2::new(400.0, 200.0), 1.0).width(), 320.0);
        assert_eq!(picture_rect(Vec2::new(2560.0, 1440.0), 2.0).width(), 512.0);
    }

    #[test]
    fn oscillator_matches_the_stored_matrix_scale() {
        // VictoryOverlayOSC: U rate 0 never moves; V amplitude 0.01.
        let p = Picture { rate: Vec2::new(0.0, 10.0), amplitude: Vec2::new(0.01, 0.01), ..default() };
        for i in 0..100 {
            let o = oscillator_offset(&p, i as f32 * 0.013);
            assert_eq!(o.x, 0.0);
            assert!(o.y.abs() <= 0.01 + 1e-6);
        }
        // A quarter period of 10 per second: the full amplitude.
        assert!((oscillator_offset(&p, 0.025).y - 0.01).abs() < 1e-5);
    }
}
