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

/// Reliable, ordered: lobby requests must all arrive, in order.
pub struct LobbyChannel;

pub fn register(app: &mut App) {
    app.component::<NetPlayer>().replicate();
    app.component::<NetGame>().replicate();
    app.register_message::<LobbyRequest>().add_direction(NetworkDirection::ClientToServer);
    app.add_channel::<LobbyChannel>(ChannelSettings { mode: ChannelMode::OrderedReliable(ReliableSettings::default()), ..default() })
        .add_direction(NetworkDirection::ClientToServer);
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
