//! Zed time (milestone 13): KF's slow motion. KFGameType.DramaticEvent
//! rolls for it on kills, headshot kills and multi-kill explosions; the
//! Patriarch's death forces it. The whole game then runs at 0.2 of normal
//! speed (Bevy's virtual clock; mouse look stays at full speed, as in KF)
//! and eases back over the last sixth. See DESIGN.md, "Zed time".

use std::collections::HashMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::engine::runlog;
use crate::game::perks::Vet;

/// KFGameType ZEDTimeDuration and ZedTimeSlomoScale.
const ZED_TIME_DURATION: f32 = 3.0;
const SLOMO_SCALE: f32 = 0.2;
/// The share of the duration over which the speed eases back.
const SPEED_UP_SHARE: f32 = 0.166;

/// KFGameType.DramaticEvent(BaseZedTimePossibility, DesiredZedTimeDuration).
#[derive(Message, Clone, Copy, Debug)]
pub struct DramaticEvent {
    pub chance: f32,
    /// 0: ZEDTimeDuration.
    pub duration: f32,
    /// For the log.
    pub reason: &'static str,
    /// Network game, host: the player whose kill or request it is (None:
    /// this game's own player). For the log.
    pub peer: Option<u64>,
}

impl DramaticEvent {
    pub fn new(chance: f32, reason: &'static str) -> Self {
        DramaticEvent { chance, duration: 0.0, reason, peer: None }
    }
}

/// The reasons a client may send (a `&'static str` again on the host).
pub fn known_reason(reason: &str) -> &'static str {
    match reason {
        "blast_4" => "blast_4",
        "blast_2" => "blast_2",
        "headshot_kill" => "headshot_kill",
        "hotkey" => "hotkey",
        "test" => "test",
        _ => "remote",
    }
}

/// Who decides zed time (multiplayer; docs/multiplayer-prototype.md,
/// "Shared zed time"). KF: KFGameType runs only on the server; clients get
/// Level.TimeDilation and ClientEnterZedTime / ClientExitZedTime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZedTimeRole {
    /// Single player: this game decides (unchanged).
    #[default]
    Local,
    /// Network host: decides for everyone, also from the clients' kills.
    Host,
    /// Network client: asks the host and follows what it says.
    Client,
}

/// What the host tells the clients (KF: SetGameSpeed, replicated as
/// Level.TimeDilation, plus ClientEnterZedTime / ClientExitZedTime).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ZedTimeCommand {
    /// DramaticEvent started or extended zed time: `left` is
    /// CurrentZEDTimeDuration.
    Start { left: f32, reason: String },
    /// bSpeedingBackUp: ClientExitZedTime (the client eases back itself).
    SpeedUp { left: f32 },
    /// CurrentZEDTimeDuration ran out: normal speed.
    End,
}

/// The link between zed time and the network code (net/zedtime.rs fills
/// and empties it). All empty in single player.
#[derive(Resource, Default)]
pub struct ZedTimeNet {
    pub role: ZedTimeRole,
    /// Host: the other players' perks and pawn centres (Bevy), by peer id,
    /// for KFGameType.Killed (the killer's ZedTimeExtensions and distance).
    pub players: HashMap<u64, (Vet, Option<Vec3>)>,
    /// Host: commands to send to every client.
    pub outgoing: Vec<ZedTimeCommand>,
    /// Client: rolls to ask the host for (explosions, test, F2).
    pub requests: Vec<DramaticEvent>,
    /// Client: the host's commands, in order.
    pub incoming: Vec<ZedTimeCommand>,
}

/// The zed time systems (the network code runs around them).
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ZedTimeSystems {
    /// KFGameType.Killed's rolls.
    Roll,
    /// DramaticEvent, Tick and the game speed.
    Run,
}

#[derive(Resource)]
pub struct ZedTime {
    /// bZEDTimeActive.
    pub active: bool,
    /// CurrentZEDTimeDuration (counts 1.1 x real seconds).
    left: f32,
    /// bSpeedingBackUp. KF eases back over the last 16.6% of
    /// ZEDTimeDuration (3 s) whatever the event's own length.
    speeding_back_up: bool,
    /// LastZedTimeEvent, in game (virtual) seconds; starts at 0.
    last_event: f32,
    /// GameSpeed (1 normal).
    speed: f32,
    /// bHadZED (KF keeps it in the player's ini; ours: once per run).
    had_zed: bool,
    rng: u32,
    /// ZedTimeExtensionsUsed (the perk's ZedTimeExtensions), reset when zed
    /// time ends.
    extensions_used: u32,
}

impl Default for ZedTime {
    fn default() -> Self {
        // A fixed seed, as elsewhere in the project (runs repeat exactly);
        // its own, so its rolls do not mirror the wave game's.
        ZedTime { active: false, left: 0.0, speeding_back_up: false, last_event: 0.0, speed: 1.0, had_zed: false, rng: 0x6c07_8965, extensions_used: 0 }
    }
}

impl ZedTime {
    /// The game speed zed time asks for (1.0 outside zed time).
    pub fn speed(&self) -> f32 {
        self.speed
    }

    fn frand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// DramaticEvent: the cooldown, the chance raised after a long wait,
    /// the roll. Returns the outcome for the log; true if it started.
    fn dramatic_event(&mut self, e: DramaticEvent, now: f32) -> (bool, String) {
        let since = now - self.last_event;
        // Don't go in slomo if we were just IN slomo.
        if since < 10.0 && e.chance != 1.0 {
            return (false, format!("refused=cooldown since={since:.1}"));
        }
        let mut chance = e.chance;
        if since > 60.0 {
            chance *= 4.0;
        } else if since > 30.0 {
            chance *= 2.0;
        }
        let roll = self.frand();
        if roll > chance {
            return (false, format!("refused=roll roll={roll:.3} chance={chance:.3}"));
        }
        self.start(if e.duration != 0.0 { e.duration } else { ZED_TIME_DURATION }, now);
        (true, format!("roll={roll:.3} chance={chance:.3}"))
    }

    fn start(&mut self, duration: f32, now: f32) {
        self.active = true;
        self.speeding_back_up = false;
        self.last_event = now;
        self.left = duration;
        self.speed = SLOMO_SCALE;
    }

    /// KFGameType.Tick: counts down at 1.1 x real time; eases the speed
    /// back over the last 16.6% of ZEDTimeDuration; ends at 0. Returns
    /// "speed_up" or "end" when those happen.
    /// Normal speed at once (a client told by the host that it ended).
    fn end(&mut self) {
        self.active = false;
        self.speeding_back_up = false;
        self.speed = 1.0;
        self.extensions_used = 0;
    }

    fn tick(&mut self, real_dt: f32) -> Option<&'static str> {
        if !self.active {
            return None;
        }
        // TrueTimeFactor = 1.1 / TimeDilation: game delta x that = 1.1 x real.
        self.left -= real_dt * 1.1;
        let ease = ZED_TIME_DURATION * SPEED_UP_SHARE;
        let mut what = None;
        if self.left < ease && self.left > 0.0 {
            if !self.speeding_back_up {
                self.speeding_back_up = true;
                what = Some("speed_up");
            }
            // Lerp(Alpha, A, B) = A + Alpha x (B - A).
            self.speed = 1.0 + (self.left / ease) * (SLOMO_SCALE - 1.0);
        }
        if self.left <= 0.0 {
            self.end();
            what = Some("end");
        }
        what
    }
}

/// The explosions' HurtRadius (Nade, LAWProj, M79GrenadeProjectile,
/// PipeBombProjectile, ...): 4 or more zeds killed 0.05, 2 or more 0.03.
pub fn blast_event(zeds_killed: usize) -> Option<DramaticEvent> {
    if zeds_killed >= 4 {
        Some(DramaticEvent::new(0.05, "blast_4"))
    } else if zeds_killed >= 2 {
        Some(DramaticEvent::new(0.03, "blast_2"))
    } else {
        None
    }
}

pub struct ZedTimePlugin;

impl Plugin for ZedTimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZedTime>()
            .init_resource::<ZedTimeNet>()
            .add_message::<DramaticEvent>()
            .configure_sets(Update, ZedTimeSystems::Roll.before(ZedTimeSystems::Run))
            .add_systems(Update, (kills_roll.in_set(ZedTimeSystems::Roll), run_zed_time.in_set(ZedTimeSystems::Run)));
    }
}

/// KFGameType.Killed (a zed the player killed, more than 0.1 s after the
/// last event: 0.05 within 3 m, else 0.025), then KFMonster.TakeDamage's
/// headshot kill (0.03, any killer), each zed once. On a network host the
/// killer may be another player (`Zed::net.damaged_by`, KF's Killer): their
/// perk and their pawn count. A client does not roll (its zeds are the
/// host's puppets; the host rolls when it applies the client's hit).
fn kills_roll(
    time: Res<Time>,
    mut zt: ResMut<ZedTime>,
    net: Res<ZedTimeNet>,
    vet: Res<crate::game::perks::Veterancy>,
    mut zeds: Query<&mut crate::zeds::zed::Zed>,
    player: Query<(&Transform, Option<&crate::player::walk::Walker>), With<crate::engine::camera::FlyCamera>>,
    mut events: MessageWriter<DramaticEvent>,
) {
    let player_at = player.single().ok().map(|(t, w)| w.map_or(t.translation, |w| w.center));
    for mut z in &mut zeds {
        if !z.is_dead() || z.zed_time_rolled {
            continue;
        }
        z.zed_time_rolled = true;
        if net.role == ZedTimeRole::Client {
            continue;
        }
        // The killer: this game's player, or (host) the player whose hit it was.
        let peer = z.net.damaged_by;
        let (killer_vet, killer_at) = match peer {
            None => (vet.vet, player_at),
            Some(p) => net.players.get(&p).copied().unwrap_or((Vet::default(), None)),
        };
        // KFGameType.Killed: during zed time a kill forces DramaticEvent(1.0)
        // while the killer's perk has ZedTimeExtensions left.
        let extensions = killer_vet.zed_time_extensions();
        if z.killed_by_player && zt.active && extensions > zt.extensions_used {
            zt.extensions_used += 1;
            let by = peer.map_or(String::new(), |p| format!(" peer={p}"));
            runlog::kv("perk_mod", &format!("kind=zed_time_extension perk={} used={} of={extensions} zed={}{by}", killer_vet.label(), zt.extensions_used, z.id));
            events.write(DramaticEvent { peer, ..DramaticEvent::new(1.0, "perk_extension") });
        } else if z.killed_by_player && time.elapsed_secs() - zt.last_event > 0.1 {
            // VSizeSquared(Killer.Pawn.Location - KilledPawn.Location) < 22500.
            let near = killer_at.is_some_and(|p| (p - z.centre).length() / crate::engine::coords::SCALE < 150.0);
            events.write(DramaticEvent { peer, ..DramaticEvent::new(if near { 0.05 } else { 0.025 }, if near { "kill_near" } else { "kill" }) });
        }
        if z.headshot_kill {
            events.write(DramaticEvent { peer, ..DramaticEvent::new(0.03, "headshot_kill") });
        }
    }
}

/// Wall-clock seconds (Unix time), to compare two games' logs.
fn wall() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn run_zed_time(
    mut zt: ResMut<ZedTime>,
    mut net: ResMut<ZedTimeNet>,
    mut events: MessageReader<DramaticEvent>,
    mut boss_died: MessageReader<crate::game::waves::BossDied>,
    real: Res<Time<Real>>,
    mut virt: ResMut<Time<Virtual>>,
    mut messages: MessageWriter<crate::game::hud::LocalMessage>,
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    keys: Res<ButtonInput<KeyCode>>,
    mut sounds: MessageWriter<crate::audio::mixer::PlaySound>,
) {
    let now = virt.elapsed_secs();
    let role = net.role;
    // Network games: the wall clock in the zed_time lines (two games' logs).
    let wall_note = if role == ZedTimeRole::Local { String::new() } else { format!(" wall={:.3} role={role:?}", wall()) };
    let mut started = None;
    // Debug: F2 or the test action "zed_time" force it (DramaticEvent(1.0),
    // which skips the 10 s rule).
    let forced = if keys.just_pressed(KeyCode::F2) {
        Some(DramaticEvent::new(1.0, "hotkey"))
    } else {
        script.0.iter().any(|(f, a)| *f == frames.0 && a == "zed_time").then(|| DramaticEvent::new(1.0, "test"))
    };
    if role == ZedTimeRole::Client {
        // The host decides: explosions' rolls and the debug ones go to it.
        for e in events.read().copied().chain(forced) {
            runlog::kv("zed_time_request", &format!("reason={} chance={} duration={}{wall_note}", e.reason, e.chance, e.duration));
            net.requests.push(e);
        }
        // The Patriarch dies on the host (its DoBossDeath).
        boss_died.read().count();
        for c in std::mem::take(&mut net.incoming) {
            match c {
                ZedTimeCommand::Start { left, reason } => {
                    zt.start(left, now);
                    started = Some((reason, zt.left));
                }
                ZedTimeCommand::SpeedUp { left } => {
                    runlog::kv("zed_time_host", &format!("event=speed_up host_left={left:.3} my_left={:.3} game_time={now:.2} real_time={:.2}{wall_note}", zt.left, real.elapsed_secs()));
                }
                ZedTimeCommand::End => {
                    runlog::kv("zed_time_host", &format!("event=end was_active={} my_left={:.3} game_time={now:.2} real_time={:.2}{wall_note}", zt.active, zt.left, real.elapsed_secs()));
                    if zt.active {
                        zt.end();
                        runlog::kv("zed_time", &format!("event=end by=host game_time={now:.2} real_time={:.2}{wall_note}", real.elapsed_secs()));
                    }
                }
            }
        }
    } else {
        for e in events.read().copied().chain(forced) {
            let (ok, how) = zt.dramatic_event(e, now);
            let by = e.peer.map_or(String::new(), |p| format!(" peer={p}"));
            runlog::kv("dramatic_event", &format!("reason={} chance={} started={ok} {how}{by}", e.reason, e.chance));
            if ok {
                started = Some((e.reason.to_string(), zt.left));
            }
        }
        // KFGameType.DoBossDeath: forced, twice as long.
        if boss_died.read().count() > 0 {
            zt.start(ZED_TIME_DURATION * 2.0, now);
            started = Some(("boss_death".to_string(), zt.left));
        }
    }
    if let Some((reason, duration)) = started {
        runlog::kv("zed_time", &format!("event=start reason={reason} duration={duration} game_time={now:.2} real_time={:.2}{wall_note}", real.elapsed_secs()));
        if role == ZedTimeRole::Host {
            net.outgoing.push(ZedTimeCommand::Start { left: duration, reason });
        }
        // ClientEnterZedTime: Zedtime_Enter, SLOT_Talk, 2.0, radius 500,
        // pitch 1.1 / Level.TimeDilation (normal pitch once the mixer
        // scales it by the game speed; ours runs at 1.0, not 1.1).
        sounds.write(zed_time_sound("KF_PlayerGlobalSnd.Zedtime_Enter", zt.speed));
        // CheckZEDMessage.
        if !zt.had_zed {
            zt.had_zed = true;
            messages.write(crate::game::hud::LocalMessage::new(crate::game::hud::MessageClass::Waiting, 5));
        }
    }
    let left = zt.left;
    if let Some(what) = zt.tick(real.delta_secs()) {
        // speed_up: ClientExitZedTime: Zedtime_Exit, as the enter sound.
        if what == "speed_up" {
            sounds.write(zed_time_sound("KF_PlayerGlobalSnd.Zedtime_Exit", zt.speed));
        }
        if role == ZedTimeRole::Host {
            net.outgoing.push(if what == "end" { ZedTimeCommand::End } else { ZedTimeCommand::SpeedUp { left } });
        }
        runlog::kv("zed_time", &format!("event={what} game_time={now:.2} real_time={:.2}{wall_note}", real.elapsed_secs()));
    }
    if virt.relative_speed() != zt.speed {
        virt.set_relative_speed(zt.speed);
    }
}

/// KFPlayerController's zed time sounds (played on the pawn's weapon or
/// the controller: at the player, an actor of its own for the slots).
fn zed_time_sound(sound: &str, speed: f32) -> crate::audio::mixer::PlaySound {
    crate::audio::mixer::PlaySound::new(sound, crate::audio::mixer::Emitter::Listener)
        .slot(crate::audio::mixer::Slot::Talk)
        .volume(2.0)
        .radius(500.0)
        .pitch(1.0 / speed.max(0.05))
        .actor(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lasts_three_over_one_point_one_real_seconds_and_eases_back() {
        let mut zt = ZedTime::default();
        zt.start(ZED_TIME_DURATION, 0.0);
        assert_eq!(zt.speed, 0.2);
        let dt = 0.001;
        let (mut t, mut speed_up_at) = (0.0, None);
        while zt.active {
            if zt.tick(dt) == Some("speed_up") {
                speed_up_at = Some(t);
            }
            t += dt;
        }
        // 3 / 1.1 = 2.727 real seconds; easing starts with 0.498 left,
        // 0.453 real seconds before the end.
        assert!((t - 2.727).abs() < 0.01, "{t}");
        assert!((speed_up_at.unwrap() - (2.727 - 0.453)).abs() < 0.01);
        assert_eq!(zt.speed, 1.0);
    }

    #[test]
    fn speed_eases_from_a_fifth_to_one() {
        let mut zt = ZedTime::default();
        zt.start(ZED_TIME_DURATION, 0.0);
        // Half way through the easing: 0.6.
        zt.left = ZED_TIME_DURATION * SPEED_UP_SHARE / 2.0 + 1e-4 * 1.1;
        zt.tick(1e-4);
        assert!((zt.speed - 0.6).abs() < 1e-3, "{}", zt.speed);
    }

    #[test]
    fn cooldown_and_long_wait_bonus() {
        let mut zt = ZedTime::default();
        // Within 10 s of the last event (0 at the start): refused.
        assert!(!zt.dramatic_event(DramaticEvent::new(0.99, "t"), 5.0).0);
        // A forced one (chance 1) still starts.
        assert!(zt.dramatic_event(DramaticEvent::new(1.0, "t"), 5.0).0);
        // 61 s later: x 4, so 0.25 always passes.
        zt.active = false;
        assert!(zt.dramatic_event(DramaticEvent::new(0.25, "t"), 66.0).0);
    }
}
