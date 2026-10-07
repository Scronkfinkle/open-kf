//! What the games send each other. Server and clients must register the
//! same things in the same order (lightyear checks it when a client
//! connects); change `PROTOCOL_ID` in mod.rs when this file changes.

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// One player's public record, made by the server for every connected
/// player and copied to everyone (KF's KFPlayerReplicationInfo:
/// PlayerName, ClientVeteranSkill / ClientVeteranSkillLevel,
/// bReadyToPlay; the character is the URL's `Character=` option).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct NetPlayer {
    /// The player's network id: 0 for the host, else the client's netcode id.
    pub peer: u64,
    pub name: String,
    /// Perk index 0-6 (`Perk::ALL`), None: no perk.
    pub perk: Option<u8>,
    pub level: u8,
    pub ready: bool,
    pub character: String,
}

/// The game's public state (KF's KFGameReplicationInfo): one entity, made
/// by the server, copied to everyone.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct NetGame {
    /// The host's `--map` (clients must have loaded the same one).
    pub map: String,
    /// `--mode` and `--length` of the host, for the log (waves are not
    /// shared yet).
    pub mode: String,
    pub length: String,
    /// GRI.bMatchHasBegun.
    pub match_started: bool,
    /// KFGRI.LobbyTimeout: seconds left of the auto-start countdown,
    /// -1 or 0: none ("Waiting for players to be ready...").
    pub lobby_timeout: i32,
}

/// A client's lobby choices, sent to the server whenever one changes
/// (KF: SendSelectedVeterancyToServer, ServerRestartPlayer /
/// ServerUnreadyPlayer, the name and character from the URL). The whole
/// state each time, so a message can never be applied out of context.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct LobbyRequest {
    pub name: String,
    pub perk: Option<u8>,
    pub level: u8,
    pub ready: bool,
    pub character: String,
}

/// One player's pawn as its owner sees it (step 2, client-authoritative):
/// what is needed to draw it on the other screens (KF replicates the same
/// things for other players' pawns: Location, Velocity, Physics, Rotation
/// and ViewPitch, the weapon attachment's FlashCount / FiringMode,
/// AnimAction, TakeHitLocation, Health). Bevy space (metres) and radians,
/// as `PawnState`. Sent by each client about 20 times a second; the
/// server copies it into the player's `NetPawn`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PawnUpdate {
    /// Counts up per update (the receiver drops older ones).
    pub seq: u32,
    /// The sender's clock (real seconds since it started) when it was
    /// sent: the receiver places the update on this time line.
    pub time: f64,
    /// There is a walking pawn (false while flying or in the lobby).
    pub active: bool,
    pub location: [f32; 3],
    pub velocity: [f32; 3],
    pub on_ground: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub weapon_class: Option<String>,
    pub flash_count: u32,
    pub firing: bool,
    pub firing_mode: u8,
    pub reloads: u32,
    pub hits: u32,
    pub hit_from: Option<[f32; 3]>,
    pub dead: bool,
    /// For the scoreboard (KF's PRI Kills, Score, Deaths and
    /// KFPRI.PlayerHealth), as this player's game has them (step 4).
    pub kills: u32,
    pub dosh: i32,
    pub deaths: u32,
    pub health: i32,
}

/// A player's pawn on the server, copied to everyone (KF's Pawn, as other
/// players' games see it). One entity per player once their match has
/// started; despawned when they leave.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetPawn {
    /// The owner (`NetPlayer::peer`).
    pub peer: u64,
    pub state: PawnUpdate,
}

/// The host's wave state (KF's KFGameReplicationInfo wave fields), on
/// the `NetGame` entity, copied to everyone when it changes (step 3).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetWave(pub crate::game::waves::WaveShare);

/// Every zed the host runs at one moment (step 3), sent to each client
/// 20 times a second. `time`: the host's clock (real seconds since it
/// started), for the client's smoothing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ZedSnapshot {
    pub seq: u32,
    pub time: f64,
    pub zeds: Vec<crate::zeds::zed::ZedNet>,
}

/// Something a host zed did to a client's player (step 3): applied on
/// that client's game (health stays per machine).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    /// A `PlayerDamaged` (zed melee, pounce, scream, Patriarch hits).
    Hurt { amount: f32, zed_id: u32, kind: crate::game::combat::HurtKind, armor_stops: bool, dam_type: crate::game::combat::DamType, source: Option<[f32; 3]> },
    /// A `PlayerPush` (Unreal units of momentum).
    Push { momentum: [f32; 3] },
    /// A Clot's grab (ZombieClot GrappleDuration).
    Grab { seconds: f32, zed_id: u32 },
}

/// The host credits a client with a kill (KF: ScoreKill on that player's
/// PRI): their kill count and dosh go up on their game (step 3).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KillCredit {
    pub zed_id: u32,
    pub scoring_value: f32,
    pub headshot: bool,
}

/// The host's choice of start spot for a player (step 4: GameInfo
/// FindPlayerStart), and with `respawn` a dead player's new pawn (KF's
/// wave-end ServerReStartPlayer). Unreal units; yaw 65536 a turn.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerStartMsg {
    pub location: [f32; 3],
    pub yaw: i32,
    pub respawn: bool,
    pub reason: String,
}

/// A projectile a host zed fired (step 4), for clients to show a harmless
/// copy (the host's own copy does the damage). Unreal units.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ProjectileFx {
    /// A Bloat glob (KFBloatVomit): start and velocity.
    Bile { at: [f32; 3], velocity: [f32; 3], zed_id: u32 },
    /// A Husk fireball (rocket: false) or the Patriarch's rocket: start
    /// and direction.
    Fireball { at: [f32; 3], dir: [f32; 3], zed_id: u32, rocket: bool },
}

/// Every door's state on the host (step 4, world/door.rs `DoorNet`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DoorStates(pub Vec<crate::world::door::DoorNetState>);

/// Every pickup shown on the host (game/pickups), sent when the list
/// changes and every 2 s.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PickupStates(pub Vec<crate::game::pickups::ShownPickup>);

/// Reliable, ordered: lobby requests must all arrive, in order.
pub struct LobbyChannel;

/// Unreliable, sequenced: zed snapshots (a lost one is replaced by the next).
pub struct ZedChannel;

/// Reliable, ordered, both ways: hits on zeds (client to host), hits on
/// players and kill credits (host to client).
pub struct GameChannel;

/// Unreliable, sequenced: pawn updates. A lost one is replaced by the
/// next; an old one arriving late is dropped.
pub struct PawnChannel;

pub fn register(app: &mut App) {
    app.component::<NetPlayer>().replicate();
    app.component::<NetGame>().replicate();
    app.component::<NetPawn>().replicate();
    app.component::<NetWave>().replicate();
    app.register_message::<LobbyRequest>().add_direction(NetworkDirection::ClientToServer);
    app.register_message::<PawnUpdate>().add_direction(NetworkDirection::ClientToServer);
    app.register_message::<ZedSnapshot>().add_direction(NetworkDirection::ServerToClient);
    app.register_message::<PlayerEvent>().add_direction(NetworkDirection::ServerToClient);
    app.register_message::<KillCredit>().add_direction(NetworkDirection::ServerToClient);
    app.register_message::<crate::game::combat::NetHit>().add_direction(NetworkDirection::ClientToServer);
    app.register_message::<PlayerStartMsg>().add_direction(NetworkDirection::ServerToClient);
    app.register_message::<ProjectileFx>().add_direction(NetworkDirection::ServerToClient);
    app.register_message::<DoorStates>().add_direction(NetworkDirection::ServerToClient);
    app.register_message::<crate::world::door::DoorRequest>().add_direction(NetworkDirection::ClientToServer);
    app.register_message::<crate::game::zed_time::ZedTimeCommand>().add_direction(NetworkDirection::ServerToClient);
    app.register_message::<super::zedtime::ZedTimeRequest>().add_direction(NetworkDirection::ClientToServer);
    app.register_message::<PickupStates>().add_direction(NetworkDirection::ServerToClient);
    app.register_message::<crate::game::pickups::PickupRequest>().add_direction(NetworkDirection::ClientToServer);
    app.register_message::<crate::game::pickups::PickupNotice>().add_direction(NetworkDirection::ServerToClient);
    app.register_message::<crate::game::pickups::DropRequest>().add_direction(NetworkDirection::ClientToServer);
    app.add_channel::<LobbyChannel>(ChannelSettings { mode: ChannelMode::OrderedReliable(ReliableSettings::default()), ..default() })
        .add_direction(NetworkDirection::ClientToServer);
    app.add_channel::<PawnChannel>(ChannelSettings { mode: ChannelMode::SequencedUnreliable, ..default() })
        .add_direction(NetworkDirection::ClientToServer);
    app.add_channel::<ZedChannel>(ChannelSettings { mode: ChannelMode::SequencedUnreliable, ..default() })
        .add_direction(NetworkDirection::ServerToClient);
    app.add_channel::<GameChannel>(ChannelSettings { mode: ChannelMode::OrderedReliable(ReliableSettings::default()), ..default() })
        .add_direction(NetworkDirection::Bidirectional);
}

/// KF: a name is cut to 20 characters in the lobby (Left(PlayerName, 20));
/// we also cut what the server stores, and an empty name becomes KF's
/// default "Player".
pub fn clean_name(name: &str) -> String {
    let n: String = name.trim().chars().filter(|c| !c.is_control()).take(20).collect();
    let n = n.trim_end().to_string();
    if n.is_empty() { "Player".into() } else { n }
}

impl NetPlayer {
    /// Applies a request; returns what changed (for the log), empty if nothing.
    pub fn apply(&mut self, req: &LobbyRequest, match_started: bool) -> Vec<String> {
        let mut changed = Vec::new();
        let name = clean_name(&req.name);
        if self.name != name {
            changed.push(format!("name=\"{name}\""));
            self.name = name;
        }
        let perk = req.perk.filter(|p| *p < 7);
        if self.perk != perk {
            changed.push(format!("perk={perk:?}"));
            self.perk = perk;
        }
        let level = req.level.min(6);
        if self.level != level {
            changed.push(format!("level={level}"));
            self.level = level;
        }
        // After the start a ready player is in the game: there is no
        // lobby to un-ready from (KF closes the menu).
        let ready = req.ready || (match_started && self.ready);
        if self.ready != ready {
            changed.push(format!("ready={ready}"));
            self.ready = ready;
        }
        if self.character != req.character {
            changed.push(format!("character={}", req.character));
            self.character = req.character.clone();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player() -> NetPlayer {
        NetPlayer { peer: 5, name: "Player".into(), perk: None, level: 0, ready: false, character: String::new() }
    }

    #[test]
    fn apply_reports_changes_and_clamps() {
        let mut p = player();
        let req = LobbyRequest { name: "  A very long player name indeed  ".into(), perk: Some(9), level: 9, ready: true, character: "Mr_Foster".into() };
        let changed = p.apply(&req, false);
        assert_eq!(p.name, "A very long player n");
        assert_eq!((p.perk, p.level, p.ready), (None, 6, true));
        assert_eq!(changed.len(), 4);
        assert!(p.apply(&req, false).is_empty());
    }

    #[test]
    fn no_unready_after_the_start() {
        let mut p = player();
        p.ready = true;
        p.apply(&LobbyRequest { ready: false, ..default() }, true);
        assert!(p.ready);
        p.apply(&LobbyRequest { ready: false, ..default() }, false);
        assert!(!p.ready);
    }
}
