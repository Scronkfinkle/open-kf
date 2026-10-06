//! The trader's radio lines (milestone 10, S5e): KFVoicePack's 'TRADER'
//! messages 0-6, sent by the wave game (game/waves.rs) at KFGameType's moments
//! and filtered as KFPlayerController.ClientLocationalVoiceMessage. See
//! DESIGN.md, "Sound and music".
//!
//! Playback (KFVoicePack.SetClientTraderMessage): the radio beep at once
//! (Walkie_Beep, SLOT_Talk, volume 10, pitch 1.1 / TimeDilation), the line
//! 0.6 s later (SLOT_Interface, ShoutVolume 2, bNoOverride, pitch 1.1 /
//! TimeDilation, not attenuated); the text line's TeamMessage also plays
//! the chat beep (KFPlayerController.PlayBeepSound: bullethitflesh2).

use bevy::prelude::*;

use crate::audio::mixer::{Emitter, PlaySound, Slot};
use crate::engine::runlog;

/// KFVoicePack.TraderSound[0..6].
const LINES: [&str; 7] = [
    "KF_Trader.Radio_Moving",
    "KF_Trader.Radio_AlmostOpen",
    "KF_Trader.Radio_ShopsOpen",
    "KF_Trader.Radio_LastWave",
    "KF_Trader.Radio_ThirtySeconds",
    "KF_Trader.Radio_TenSeconds",
    "KF_Trader.Radio_Closed",
];
/// The trader's lines are their own "actor" for the slots.
const ACTOR: u64 = 5;

/// ServerSpeech / ClientLocationalVoiceMessage('TRADER', n).
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraderSpeech(pub u8);

pub struct TraderVoicePlugin;

impl Plugin for TraderVoicePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TraderSpeech>().add_systems(Update, trader_voice);
    }
}

#[derive(Default)]
struct State {
    /// bHasHeardTraderWelcomeMessage: set on entering a shop (ShopVolume
    /// touch; the Welcome line itself has no sender and plays nothing, by
    /// our reading), cleared by the "closed" line.
    heard_welcome: bool,
    was_inside: bool,
    /// Lines waiting for their 0.6 s timer: (game seconds, line).
    pending: Vec<(f32, u8)>,
}

fn trader_voice(
    time: Res<Time>,
    speed: Res<Time<Virtual>>,
    mut requests: MessageReader<TraderSpeech>,
    shops: Res<crate::game::trader::Shops>,
    mut out: MessageWriter<PlaySound>,
    mut st: Local<State>,
    (mut preload, mut preloaded): (MessageWriter<crate::audio::mixer::PreloadSounds>, Local<bool>),
) {
    if !*preloaded {
        *preloaded = true;
        let mut sounds: Vec<String> = LINES.iter().map(|s| s.to_string()).collect();
        sounds.extend(["KF_Trader.Walkie_Beep", "KFWeaponSound.bullethitflesh2", "KF_Trader.TooExpensive", "KF_Trader.TooHeavy", "KF_InventorySnd.Vest_Pickup"].map(String::from));
        preload.write(crate::audio::mixer::PreloadSounds { what: "trader".into(), sounds });
    }
    let now = time.elapsed_secs();
    // Pitch 1.1 / TimeDilation: normal pitch once the mixer scales it by
    // the game speed (ours runs at 1.0, not 1.1).
    let pitch = 1.0 / speed.relative_speed().max(0.05);
    let inside = shops.player_inside().is_some();
    if inside && !st.was_inside {
        st.heard_welcome = true;
    }
    st.was_inside = inside;
    for &TraderSpeech(id) in requests.read() {
        // ClientLocationalVoiceMessage's filters: in a shop the 30 s line is
        // dropped and the 10 s line kept; outside, the 10 s line is dropped,
        // and the 30 s line too once the player has been in a shop.
        let skip = match id {
            4 => inside || st.heard_welcome,
            5 => !inside,
            _ => false,
        };
        if id == 6 {
            st.heard_welcome = false;
        }
        runlog::kv("trader_speech", &format!("line={id} sound={} played={} inside={inside}", LINES.get(id as usize).unwrap_or(&"?"), !skip));
        if skip || id as usize >= LINES.len() {
            continue;
        }
        out.write(PlaySound::new("KF_Trader.Walkie_Beep", Emitter::Listener).slot(Slot::Talk).volume(10.0).pitch(pitch).actor(ACTOR));
        out.write(PlaySound::new("KFWeaponSound.bullethitflesh2", Emitter::Listener).actor(ACTOR));
        st.pending.push((now + 0.6, id));
    }
    let due: Vec<u8> = st.pending.iter().filter(|(t, _)| now >= *t).map(|&(_, id)| id).collect();
    st.pending.retain(|(t, _)| now < *t);
    for id in due {
        out.write(PlaySound::new(LINES[id as usize], Emitter::Listener).slot(Slot::Interface).volume(2.0).pitch(pitch).actor(ACTOR).no_override());
    }
}
