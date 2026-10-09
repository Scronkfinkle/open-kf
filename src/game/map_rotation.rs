//! The map list ("map rotation") and the map-vote switches: which map the
//! game goes to after the current one. Plain logic (no Bevy systems), all
//! unit-tested. See DESIGN.md, "Map rotation and map voting without
//! restarting", step 2 (map list) and step 5 (launcher settings).
//!
//! KF's rules (Engine/MapList.uc, Engine/GameInfo.uc RestartGame; the list
//! is KFMod.KFMapList, filled from KillingFloor.ini `[KFmod.KFMaplist]`):
//! - When the list is made, entries that are empty or not installed are
//!   removed; if any were, the position (`MapNum`) goes back to 0
//!   (MapList.PreBeginPlay / HasInvalidMaps).
//! - GetNextMap: find the current map in the list (case-insensitive; KF
//!   tries the full URL, then with options reordered, then loosely, then
//!   the map name only — our names have no options, so that is one
//!   case-insensitive comparison of the bare name) and take the entry
//!   after it. Not found: the entry after `MapNum` ("blind switch").
//! - UpdateMapNum: past the end wraps to 0; entries not installed are
//!   skipped, but the walk stops when it gets back to the old `MapNum`;
//!   the new position is saved.
//! - RestartGame: an empty answer (empty list) falls back to
//!   GetMapName("KF", "", 1): the first installed map whose name starts
//!   with "KF" (native code, behaviour recalled, not checked). Nothing at
//!   all: the same map again ("?Restart").
//!
//! Step 2/3 (map change at the end of a match) call [`MapRotation::next_map`]
//! and then save the position with `launcher::save_map_rotation`.

// next_map and its helpers are called by the map-change step (not wired yet).
#![allow(dead_code)]

use bevy::prelude::Resource;

/// KF's default map list (KillingFloor.ini `[KFmod.KFMaplist]` Maps= and
/// `[DefaultKF MaplistRecord]` DefaultMaps=), in KF's order.
pub const KF_DEFAULT_MAPS: [&str; 5] = ["KF-BioticsLab", "KF-Farm", "KF-Manor", "KF-Offices", "KF-WestLondon"];

/// KF's map prefix for the fallback (GameInfo.MapPrefix for KF games).
pub const KF_MAP_PREFIX: &str = "KF";

/// KF's xVoting VoteTimeLimit (KillingFloor.ini / Default.ini: 30 s).
pub const KF_VOTE_TIME_LIMIT: u32 = 30;
/// The range we accept for the vote time (ours; KF has no limit).
pub const VOTE_TIME_MIN: u32 = 5;
pub const VOTE_TIME_MAX: u32 = 600;

/// The map list and where in it the game is (KF's `Maps` and `MapNum`).
/// Inserted at startup from `--map-list` / the settings file. Not used by
/// anything yet: step 2 (single-player map change) and step 3 (multiplayer
/// map change) call [`MapRotation::next_map`] at the end of a match.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct MapRotation {
    /// Map names, e.g. "KF-Farm" (no `.rom`, no options).
    pub maps: Vec<String>,
    /// KF's MapNum: the entry last travelled to.
    pub position: usize,
}

impl Default for MapRotation {
    fn default() -> Self {
        MapRotation { maps: KF_DEFAULT_MAPS.iter().map(|m| m.to_string()).collect(), position: 0 }
    }
}

/// The map-vote switches the launcher and the command line set (KF's
/// xVoting.xVotingHandler bMapVote and VoteTimeLimit): the two lines of
/// the settings file. Not a game resource: at startup they fill the one
/// the game reads, `game::map_vote::MapVoteSettings` ([`Self::settings`]),
/// which holds KF's other vote values too. Off by default, as in KF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapVoteConfig {
    pub enabled: bool,
    /// Seconds the vote lasts.
    pub time_limit: u32,
}

impl Default for MapVoteConfig {
    fn default() -> Self {
        MapVoteConfig { enabled: false, time_limit: KF_VOTE_TIME_LIMIT }
    }
}

impl MapVoteConfig {
    /// The game's vote settings: KF's values with these two switches.
    pub fn settings(&self) -> crate::game::map_vote::MapVoteSettings {
        crate::game::map_vote::MapVoteSettings { enabled: self.enabled, time_limit: self.time_limit, ..Default::default() }
    }
}

/// A map name without `.rom` and without `?options` (KF's
/// MaplistRecord.GetBaseMapName, which strips options and `.ut2`).
pub fn base_name(map: &str) -> &str {
    let s = map.split('?').next().unwrap_or("").trim();
    match s.len().checked_sub(4) {
        Some(i) if s.is_char_boundary(i) && s[i..].eq_ignore_ascii_case(".rom") => &s[..i],
        _ => s,
    }
}

fn same_map(a: &str, b: &str) -> bool {
    base_name(a).eq_ignore_ascii_case(base_name(b))
}

fn is_installed(map: &str, installed: &[String]) -> bool {
    installed.iter().any(|m| same_map(m, map))
}

/// A `--map-list` / `map_list=` value: names separated by commas, blanks
/// dropped, `.rom` removed. An empty text is an empty list.
pub fn parse_map_list(v: &str) -> Vec<String> {
    v.split(',').map(base_name).filter(|m| !m.is_empty()).map(str::to_string).collect()
}

/// The list as saved and passed on the command line.
pub fn map_list_text(maps: &[String]) -> String {
    maps.join(",")
}

/// A vote time in seconds (`--vote-time`, `vote_time_limit=`).
pub fn parse_vote_time(v: &str) -> Result<u32, String> {
    let n: u32 = v.trim().parse().map_err(|_| format!("not a number: {v}"))?;
    if !(VOTE_TIME_MIN..=VOTE_TIME_MAX).contains(&n) {
        return Err(format!("vote time must be {VOTE_TIME_MIN} to {VOTE_TIME_MAX} seconds: {v}"));
    }
    Ok(n)
}

/// The first installed map after `after` (alphabetically, wrapping) whose
/// name starts with `prefix`; the first one if `after` is not among them.
/// KF's native GetMapName(prefix, after, 1) as recalled (not checked).
pub fn prefix_map(prefix: &str, after: &str, installed: &[String]) -> Option<String> {
    let mut list: Vec<&String> = installed.iter().filter(|m| m.len() >= prefix.len() && m.is_char_boundary(prefix.len()) && m[..prefix.len()].eq_ignore_ascii_case(prefix)).collect();
    list.sort_by_key(|m| m.to_ascii_lowercase());
    let i = list.iter().position(|m| same_map(m, after)).map_or(0, |i| (i + 1) % list.len().max(1));
    list.get(i).map(|m| m.to_string())
}

impl MapRotation {
    pub fn new(maps: Vec<String>, position: usize) -> Self {
        MapRotation { maps, position }
    }

    /// KF's list with the position kept (SetMaplist: a position past the
    /// end goes back to 0).
    pub fn with_maps(&self, maps: Vec<String>) -> Self {
        let position = if self.position >= maps.len() { 0 } else { self.position };
        MapRotation { maps, position }
    }

    /// The launcher's check box: takes the map out of the list, or adds it
    /// at the end. The position goes back to 0 if past the end.
    pub fn toggle(&mut self, map: &str) {
        match self.maps.iter().position(|m| same_map(m, map)) {
            Some(i) => {
                self.maps.remove(i);
            }
            None => self.maps.push(base_name(map).to_string()),
        }
        if self.position >= self.maps.len() {
            self.position = 0;
        }
    }

    /// The launcher's up / down buttons: swaps entry `i` with its
    /// neighbour (`d` -1 up, 1 down); nothing at the ends.
    pub fn move_entry(&mut self, i: usize, d: i32) {
        let j = i as i64 + d as i64;
        if i < self.maps.len() && (0..self.maps.len() as i64).contains(&j) {
            self.maps.swap(i, j as usize);
        }
    }

    /// The entries that are empty or not installed are removed; if any
    /// were, the position goes back to 0 (MapList.PreBeginPlay ->
    /// HasInvalidMaps; KF saves the shortened list too). Returns how many
    /// were removed.
    pub fn drop_missing(&mut self, installed: &[String]) -> usize {
        let before = self.maps.len();
        self.maps.retain(|m| !base_name(m).is_empty() && is_installed(m, installed));
        let removed = before - self.maps.len();
        if removed > 0 {
            self.position = 0;
        }
        removed
    }

    /// The map to go to after `current` (KF's RestartGame with
    /// MapList.GetNextMap), moving the position on. `installed`: the
    /// install's maps (`list_installed_maps`). Never empty: when neither
    /// the list nor the "KF" fallback gives a map, `current` again (KF's
    /// "?Restart"). The caller saves the rotation afterwards (KF's
    /// SaveConfig); the list may have lost entries that are not installed.
    pub fn next_map(&mut self, current: &str, installed: &[String]) -> String {
        self.drop_missing(installed);
        let from_list = self.get_next_map(current, installed);
        let next = from_list.clone().or_else(|| prefix_map(KF_MAP_PREFIX, "", installed)).unwrap_or_else(|| base_name(current).to_string());
        crate::engine::runlog::kv(
            "map_rotation_next",
            &format!("current={} next={next} source={} position={} list=[{}]", base_name(current), if from_list.is_some() { "list" } else if next.eq_ignore_ascii_case(base_name(current)) { "restart" } else { "prefix_fallback" }, self.position, self.maps.join(",")),
        );
        next
    }

    /// MapList.GetNextMap: None when the list is empty.
    fn get_next_map(&mut self, current: &str, installed: &[String]) -> Option<String> {
        if base_name(current).is_empty() {
            return self.update_map_num(self.position + 1, installed);
        }
        match self.maps.iter().position(|m| same_map(m, current)) {
            Some(i) => self.update_map_num(i + 1, installed),
            // "Performing blind switch to index MapNum + 1".
            None => self.update_map_num(self.position + 1, installed),
        }
    }

    /// MapList.UpdateMapNum: wraps, skips maps that are not installed
    /// (stopping at the old position), sets the position.
    fn update_map_num(&mut self, mut n: usize, installed: &[String]) -> Option<String> {
        let len = self.maps.len();
        if len == 0 {
            return None;
        }
        loop {
            if n >= len {
                n = 0;
            }
            if n == self.position || self.position >= len {
                break;
            }
            if is_installed(&self.maps[n], installed) {
                break;
            }
            n += 1;
        }
        self.position = n;
        Some(self.maps[n].clone())
    }
}

/// The playable maps in the install's `Maps` folder: every `.rom` but KF's
/// start-up, intro and main-menu maps, sorted by name.
pub fn list_installed_maps(root: &std::path::Path) -> Vec<String> {
    let mut maps: Vec<String> = std::fs::read_dir(root.join("Maps"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".rom").or_else(|| n.strip_suffix(".ROM"))).map(str::to_string))
                .filter(|n| !["entry", "kfintro", "kf-menu"].contains(&n.to_ascii_lowercase().as_str()))
                .collect()
        })
        .unwrap_or_default();
    maps.sort_by_key(|m| m.to_ascii_lowercase());
    maps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn all_installed() -> Vec<String> {
        s(&["KF-Aperture", "KF-BioticsLab", "KF-Farm", "KF-Manor", "KF-Offices", "KF-WestLondon"])
    }

    #[test]
    fn advances_through_the_list() {
        let mut r = MapRotation::default();
        assert_eq!(r.next_map("KF-BioticsLab", &all_installed()), "KF-Farm");
        assert_eq!(r.position, 1);
        assert_eq!(r.next_map("KF-Farm", &all_installed()), "KF-Manor");
        assert_eq!(r.position, 2);
        // Case and `.rom` do not matter.
        assert_eq!(r.next_map("kf-manor.rom", &all_installed()), "KF-Offices");
    }

    #[test]
    fn wraps_after_the_last() {
        let mut r = MapRotation::default();
        assert_eq!(r.next_map("KF-WestLondon", &all_installed()), "KF-BioticsLab");
        assert_eq!(r.position, 0);
    }

    #[test]
    fn current_not_in_list_is_a_blind_switch() {
        let mut r = MapRotation { position: 2, ..Default::default() };
        // KF-Aperture is not in the list: MapNum + 1.
        assert_eq!(r.next_map("KF-Aperture", &all_installed()), "KF-Offices");
        assert_eq!(r.position, 3);
        let mut r = MapRotation { position: 4, ..Default::default() };
        assert_eq!(r.next_map("KF-Aperture", &all_installed()), "KF-BioticsLab");
    }

    #[test]
    fn missing_entries_are_dropped_and_position_reset() {
        // KF-Farm and KF-Manor not installed: removed, MapNum back to 0.
        let installed = s(&["KF-BioticsLab", "KF-Offices", "KF-WestLondon"]);
        let mut r = MapRotation { position: 3, ..Default::default() };
        assert_eq!(r.next_map("KF-BioticsLab", &installed), "KF-Offices");
        assert_eq!(r.maps, s(&["KF-BioticsLab", "KF-Offices", "KF-WestLondon"]));
        assert_eq!(r.position, 1);
    }

    #[test]
    fn update_map_num_skips_and_stops_at_old_position() {
        // The skip in UpdateMapNum on its own (the list kept whole).
        let installed = s(&["KF-BioticsLab", "KF-Offices"]);
        let mut r = MapRotation::default();
        assert_eq!(r.update_map_num(1, &installed), Some("KF-Offices".into()));
        assert_eq!(r.position, 3);
        // From 3: 4 missing, wraps to 0 (installed).
        assert_eq!(r.update_map_num(4, &installed), Some("KF-BioticsLab".into()));
        // Back at the old position: stops even if not installed.
        let mut r = MapRotation { position: 1, ..Default::default() };
        assert_eq!(r.update_map_num(2, &s(&[])), Some("KF-Farm".into()));
        assert_eq!(r.position, 1);
    }

    #[test]
    fn all_missing_falls_back_to_first_kf_map() {
        let installed = s(&["KF-Zeta", "KF-Aperture", "Other-Map"]);
        let mut r = MapRotation::default();
        assert_eq!(r.next_map("KF-Zeta", &installed), "KF-Aperture");
        assert!(r.maps.is_empty());
        assert_eq!(r.position, 0);
    }

    #[test]
    fn empty_list() {
        let mut r = MapRotation::new(vec![], 0);
        assert_eq!(r.next_map("KF-Farm", &all_installed()), "KF-Aperture");
        // Nothing installed with the prefix: the same map again.
        assert_eq!(r.next_map("KF-Farm", &s(&["Other"])), "KF-Farm");
    }

    #[test]
    fn single_entry_list_repeats() {
        let mut r = MapRotation::new(s(&["KF-Farm"]), 0);
        assert_eq!(r.next_map("KF-Farm", &all_installed()), "KF-Farm");
        assert_eq!(r.next_map("KF-Manor", &all_installed()), "KF-Farm");
    }

    #[test]
    fn list_text_and_prefix() {
        assert_eq!(parse_map_list(" KF-Farm , ,KF-Manor.rom,KF-Offices?Game=x"), s(&["KF-Farm", "KF-Manor", "KF-Offices"]));
        assert!(parse_map_list("").is_empty());
        assert_eq!(map_list_text(&s(&["KF-Farm", "KF-Manor"])), "KF-Farm,KF-Manor");
        assert_eq!(prefix_map("KF", "KF-Farm", &all_installed()), Some("KF-Manor".into()));
        assert_eq!(prefix_map("KF", "KF-WestLondon", &all_installed()), Some("KF-Aperture".into()));
        assert_eq!(prefix_map("KF", "", &s(&[])), None);
        assert_eq!(parse_vote_time("30"), Ok(30));
        assert!(parse_vote_time("2").is_err() && parse_vote_time("x").is_err());
    }

    #[test]
    fn launcher_edits() {
        let mut r = MapRotation { position: 4, ..Default::default() };
        r.toggle("kf-westlondon");
        assert_eq!((r.maps.len(), r.position), (4, 0));
        r.toggle("KF-Aperture");
        assert_eq!(r.maps.last().map(String::as_str), Some("KF-Aperture"));
        r.move_entry(4, -1);
        assert_eq!(r.maps[3], "KF-Aperture");
        r.move_entry(0, -1);
        r.move_entry(4, 1);
        assert_eq!(r.maps, s(&["KF-BioticsLab", "KF-Farm", "KF-Manor", "KF-Aperture", "KF-Offices"]));
    }

    #[test]
    fn with_maps_keeps_or_resets_position() {
        let r = MapRotation { position: 3, ..Default::default() };
        assert_eq!(r.with_maps(s(&["KF-Farm"])).position, 0);
        assert_eq!(r.with_maps(s(&["A", "B", "C", "D"])).position, 3);
    }
}
