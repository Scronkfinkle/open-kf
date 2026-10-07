//! Music (milestone 10, S6): the map's KFMusicTrigger songs, switched by
//! the wave game as KFGameType does, with KFMusicInteraction's fades. See
//! DESIGN.md, "Sound and music".
//!
//! - MatchInProgress.Timer: a wave in progress and !MusicPlaying ->
//!   StartGameMusic(true), the combat song; trader time and
//!   !CalmMusicPlaying -> StartGameMusic(false), the calm song. The song
//!   is WaveBasedSongs[WaveNum]'s if set, else CombatSong / Song.
//! - The Patriarch's entrance: ClientSetMusic(BossBattleSong,
//!   MTRAN_FastFade), fading 1 s out and 1 s in.
//! - KFMusicInteraction.SetSong: with a song playing and a fade-out time,
//!   the old song's volume goes down linearly; under 0.1 of it, it stops
//!   and the next starts (PlayMusic with the fade-in time). Otherwise the
//!   old one stops at once.
//! - The trigger's song fields are `localized`: the map's
//!   `System/<map>.int` section for the trigger ([KFMusicTrigger0])
//!   replaces what the map stores (KF-WestLondon: CombatSong KFSIN8 in the
//!   map, DirgeDisunion1 and a WaveBasedSongs list in the .int), as UE2
//!   does when it loads a localized property.
//! - Songs are `Music/<name>.ogg`. A name with no file plays nothing, as
//!   the engine's PlayMusic; logged `music_missing`.

use std::path::PathBuf;

use bevy::prelude::*;
use ue_assets::package_set::PackageSet;
use ue_assets::properties::Value;

use crate::audio::mixer::{Audio, MusicHandle};
use crate::engine::runlog;

/// KFGameType.BossBattleSong.
const BOSS_BATTLE_SONG: &str = "KF_Abandon";

/// What the wave game asks for (KFGameType.StartGameMusic and the boss's
/// ClientSetMusic). The wave number is WaveNum (0-based) at the time.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum MusicCue {
    Combat(usize),
    Calm(usize),
    Boss,
}

/// The map's KFMusicTrigger (KFGameType.MapSongHandler).
#[derive(Debug, Default, Clone)]
struct SongHandler {
    song: String,
    combat_song: String,
    fade_in: f32,
    fade_out: f32,
    /// WaveBasedSongs: (CombatSong, CalmSong) per wave.
    waves: Vec<(String, String)>,
}

impl SongHandler {
    fn pick(&self, cue: MusicCue) -> (String, f32, f32) {
        let per_wave = |w: usize, combat: bool| {
            self.waves.get(w).map(|(c, k)| if combat { c.clone() } else { k.clone() }).filter(|s| !s.is_empty())
        };
        match cue {
            MusicCue::Combat(w) => (per_wave(w, true).unwrap_or_else(|| self.combat_song.clone()), self.fade_in, self.fade_out),
            MusicCue::Calm(w) => (per_wave(w, false).unwrap_or_else(|| self.song.clone()), self.fade_in, self.fade_out),
            MusicCue::Boss => (BOSS_BATTLE_SONG.into(), 1.0, 1.0),
        }
    }
}

/// KFMusicInteraction's state.
#[derive(Resource, Default)]
struct Music {
    /// None: the map has no KFMusicTrigger (MapSongHandler None: no music).
    handler: Option<SongHandler>,
    music_dir: PathBuf,
    /// ActiveSong, and its stream (None if the file is missing).
    active: Option<(String, Option<MusicHandle>)>,
    /// bFadeOutSong: (FadeOutPos, FadeTimes[1], NextSong, FadeTimes[0]).
    fade_out: Option<(f32, f32, String, f32)>,
    /// PlayMusic's fade-in: (seconds done, seconds total).
    fade_in: Option<(f32, f32)>,
}

pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MusicCue>().init_resource::<Music>().add_systems(Startup, load_song_handler).add_systems(Update, run_music);
    }
}

fn load_song_handler(request: Res<crate::world::map::MapRequest>, mut music: ResMut<Music>) {
    music.music_dir = request.install_root.join("Music");
    let path = request.install_root.join("Maps").join(format!("{}.rom", request.map));
    let set = PackageSet::new(&request.install_root);
    let Ok(lp) = set.load_path(&path) else {
        runlog::kv("music_handler", "found=false reason=map_not_loaded");
        return;
    };
    let pkg = &lp.pkg;
    let Some(i) = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i) == "KFMusicTrigger") else {
        runlog::kv("music_handler", "found=false");
        return;
    };
    let Ok(props) = ue_assets::properties::read_export_properties_ext(pkg, i, &["WaveBasedSongs"]) else {
        runlog::kv("music_handler", "found=false reason=unreadable");
        return;
    };
    let text = |list: &ue_assets::properties::PropertyList, name: &str| match list.get(pkg, name) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let float = |name: &str| match props.get(pkg, name) {
        Some(Value::Float(f)) => *f,
        _ => 0.0,
    };
    let waves = match props.get(pkg, "WaveBasedSongs") {
        Some(Value::StructArray(items)) => items.iter().map(|e| (text(e, "CombatSong"), text(e, "CalmSong"))).collect(),
        _ => Vec::new(),
    };
    let mut h = SongHandler { song: text(&props, "Song"), combat_song: text(&props, "CombatSong"), fade_in: float("FadeInTime"), fade_out: float("FadeOutTime"), waves };
    // Localized: the .int file's values win.
    let section = pkg.object_name(ue_assets::package::ObjectRef::Export(i)).to_string();
    let int_path = request.install_root.join("System").join(format!("{}.int", request.map));
    let overrides = std::fs::read_to_string(&int_path).map(|t| int_section(&t, &section)).unwrap_or_default();
    for (key, value) in &overrides {
        match key.as_str() {
            "song" => h.song = unquote(value),
            "combatsong" => h.combat_song = unquote(value),
            "wavebasedsongs" => h.waves = parse_wave_songs(value),
            _ => {}
        }
    }
    runlog::kv("music_localized", &format!("file={}.int section={section} keys=[{}]", request.map, overrides.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(" ")));
    runlog::kv(
        "music_handler",
        &format!(
            "found=true song={} combat_song={} fade_in={} fade_out={} wave_songs=[{}]",
            h.song,
            h.combat_song,
            h.fade_in,
            h.fade_out,
            h.waves.iter().map(|(c, k)| format!("{c}/{k}")).collect::<Vec<_>>().join(" ")
        ),
    );
    music.handler = Some(h);
}

/// The `key=value` lines of one `[section]` of a localization file (keys
/// lowercase).
pub(crate) fn int_section(text: &str, section: &str) -> Vec<(String, String)> {
    let mut inside = false;
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name.eq_ignore_ascii_case(section);
        } else if inside && let Some((k, v)) = line.split_once('=') {
            out.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    out
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches('"').to_string()
}

/// `((CombatSong="A",CalmSong="B"),(...))`: one (combat, calm) per wave.
fn parse_wave_songs(v: &str) -> Vec<(String, String)> {
    let inner = v.trim().strip_prefix('(').and_then(|x| x.strip_suffix(')')).unwrap_or("");
    let mut waves = Vec::new();
    for group in inner.split(')').map(|g| g.trim_start_matches(',').trim_start_matches('(')).filter(|g| !g.trim().is_empty()) {
        let (mut combat, mut calm) = (String::new(), String::new());
        for field in group.split(',') {
            if let Some((k, val)) = field.split_once('=') {
                match k.trim().to_ascii_lowercase().as_str() {
                    "combatsong" => combat = unquote(val),
                    "calmsong" => calm = unquote(val),
                    _ => {}
                }
            }
        }
        waves.push((combat, calm));
    }
    waves
}

/// `Music/<name>.ogg`, ignoring case.
fn song_file(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let want = format!("{name}.ogg").to_ascii_lowercase();
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().to_ascii_lowercase() == want))
}

fn run_music(real: Res<Time<Real>>, mut cues: MessageReader<MusicCue>, audio: Res<Audio>, mut music: ResMut<Music>) {
    let volume = if audio.muted { 0.0 } else { audio.music_volume };
    for &cue in cues.read() {
        let Some(h) = music.handler.clone() else {
            continue;
        };
        let (song, fade_in, fade_out) = h.pick(cue);
        runlog::kv("music_cue", &format!("cue={cue:?} song={song} fade_in={fade_in} fade_out={fade_out}"));
        set_song(&mut music, &audio, song, fade_in, fade_out);
    }
    let dt = real.delta_secs();
    // KFMusicInteraction.Tick: the fade-out.
    if let Some((pos, total, next, next_fade_in)) = music.fade_out.clone() {
        let scalar = pos / total;
        if scalar < 0.1 {
            music.fade_out = None;
            if let Some((old, _)) = music.active.take() {
                runlog::kv("music_stop", &format!("song={old} reason=faded"));
            }
            if !next.is_empty() {
                start(&mut music, &audio, next, next_fade_in);
            }
        } else {
            music.fade_out = Some((pos - dt, total, next, next_fade_in));
            if let Some((_, Some(handle))) = &music.active {
                handle.set_volume(scalar * volume);
            }
        }
        return;
    }
    // PlayMusic's fade-in (native: assumed linear from 0).
    let level = match music.fade_in {
        Some((done, total)) if done < total => {
            music.fade_in = Some((done + dt, total));
            done / total
        }
        _ => {
            music.fade_in = None;
            1.0
        }
    };
    if let Some((_, Some(handle))) = &music.active {
        handle.set_volume(level * volume);
    }
}

/// KFMusicInteraction.SetSong.
fn set_song(music: &mut Music, audio: &Audio, song: String, fade_in: f32, fade_out: f32) {
    if music.active.is_some() && fade_out > 0.0 {
        music.fade_out = Some((fade_out, fade_out, song, fade_in));
        return;
    }
    if let Some((old, _)) = music.active.take() {
        runlog::kv("music_stop", &format!("song={old} reason=switch"));
    }
    music.fade_out = None;
    start(music, audio, song, fade_in);
}

/// PlayerController.PlayMusic(Song, FadeInTime).
fn start(music: &mut Music, audio: &Audio, song: String, fade_in: f32) {
    if song.is_empty() {
        return;
    }
    let handle = match song_file(&music.music_dir, &song) {
        Some(path) => audio.play_music(&path),
        None => {
            runlog::kv("music_missing", &format!("song={song}"));
            None
        }
    };
    runlog::kv("music_play", &format!("song={song} playing={} fade_in={fade_in}", handle.is_some()));
    music.fade_in = (fade_in > 0.0).then_some((0.0, fade_in));
    music.active = Some((song, handle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_localized_trigger_section() {
        let text = "[KFMusicTrigger0]\nCombatSong=\"DirgeDisunion1\"\nWaveBasedSongs=((CombatSong=\"A\",CalmSong=\"B\"),(CombatSong=\"C\",CalmSong=\"\"))\n\n[KFUseTrigger0]\nMessage=\"Press USE Key\"\n";
        let kv = int_section(text, "KFMusicTrigger0");
        assert_eq!(kv.len(), 2);
        assert_eq!(unquote(&kv[0].1), "DirgeDisunion1");
        assert_eq!(parse_wave_songs(&kv[1].1), vec![("A".into(), "B".into()), ("C".into(), String::new())]);
    }

    #[test]
    fn wave_songs_win_over_the_map_songs() {
        let h = SongHandler {
            song: "Calm".into(),
            combat_song: "Fight".into(),
            fade_in: 2.0,
            fade_out: 3.0,
            waves: vec![("W1Fight".into(), String::new()), (String::new(), "W2Calm".into())],
        };
        assert_eq!(h.pick(MusicCue::Combat(0)).0, "W1Fight");
        assert_eq!(h.pick(MusicCue::Calm(0)).0, "Calm");
        assert_eq!(h.pick(MusicCue::Combat(1)).0, "Fight");
        assert_eq!(h.pick(MusicCue::Calm(1)).0, "W2Calm");
        assert_eq!(h.pick(MusicCue::Combat(5)), ("Fight".into(), 2.0, 3.0));
        assert_eq!(h.pick(MusicCue::Boss), ("KF_Abandon".into(), 1.0, 1.0));
    }
}
