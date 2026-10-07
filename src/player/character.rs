//! The player's character: KF's character records (`System/*.upl`), the
//! default character, and the first-person sleeve texture its species
//! gives every weapon (KFWeapon.HandleSleeveSwapping).

use std::path::Path;

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::Value;

use crate::engine::runlog;

/// KFPawn.GetDefaultCharacter (also `[DefaultPlayer] Character=` in KF's
/// defuser.ini).
pub const DEFAULT_CHARACTER: &str = "Corporal_Lewis";

/// `--character NAME`: the character asked for on the command line.
#[derive(Resource, Debug, Default, Clone)]
pub struct CharacterChoice(pub Option<String>);

/// One character record (xUtil.PlayerRecord), the fields we use.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerRecord {
    pub name: String,
    /// Species class path, e.g. `KFmod.SoldierSpecies`.
    pub species: String,
}

/// The chosen character, resolved.
pub struct Character {
    pub name: String,
    /// KFSpeciesType.SleeveTexture of the species (None: weapons keep their own skins).
    pub sleeve: Option<ObjectHandle>,
}

/// The `Player=(...)` records of one .upl file. Keys are matched ignoring
/// case (files write both `species=` and `Species=`).
pub fn parse_upl(text: &str) -> Vec<PlayerRecord> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(body) = line
            .split_once('=')
            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("Player"))
            .map(|(_, v)| v.trim().trim_start_matches('(').trim_end_matches(')'))
        else {
            continue;
        };
        let (mut name, mut species) = (None, None);
        for (key, value) in fields(body) {
            if key.eq_ignore_ascii_case("DefaultName") {
                name = Some(value);
            } else if key.eq_ignore_ascii_case("Species") {
                species = Some(value);
            }
        }
        if let Some(name) = name.filter(|n| !n.is_empty()) {
            out.push(PlayerRecord {
                name,
                species: species.unwrap_or_default(),
            });
        }
    }
    out
}

/// `Key=Value,Key="quoted, value",...` split at the commas outside quotes.
fn fields(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut cur, mut quoted) = (String::new(), false);
    let mut push = |s: &str| {
        if let Some((k, v)) = s.split_once('=') {
            out.push((k.trim().to_string(), v.trim().trim_matches('"').to_string()));
        }
    };
    for c in body.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            ',' if !quoted => {
                push(&cur);
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    push(&cur);
    out
}

/// All character records in the install's System folder, sorted by name.
pub fn read_records(install_root: &Path) -> Vec<PlayerRecord> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(install_root.join("System")) else {
        return out;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("upl")) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            out.extend(parse_upl(&String::from_utf8_lossy(&bytes)));
        }
    }
    out.sort_by_key(|r| r.name.to_ascii_lowercase());
    out
}

/// xUtil.FindPlayerRecord: the record with this name, ignoring case.
pub fn find_record<'a>(records: &'a [PlayerRecord], name: &str) -> Option<&'a PlayerRecord> {
    records.iter().find(|r| r.name.eq_ignore_ascii_case(name))
}

/// Picks the character as KFPawn.Setup does: the requested record, else
/// (unknown name, no species, or a species that is not a
/// SPECIES_KFMaleHuman) the default character. Logs the choice.
pub fn choose(set: &PackageSet, defaults: &ClassDefaults, install_root: &Path, request: Option<&str>) -> Option<Character> {
    let records = read_records(install_root);
    let species_of = |r: &PlayerRecord| {
        set.find_object(&r.species, Some("Class"))
            .filter(|c| defaults.is_a(c, "SPECIES_KFMaleHuman"))
    };
    let wanted = request.unwrap_or(DEFAULT_CHARACTER);
    let picked = match find_record(&records, wanted) {
        Some(r) => match species_of(r) {
            Some(s) => Some((r, s, "")),
            None => {
                runlog::kv("character_bad_species", &format!("name={} species={}", r.name, r.species));
                None
            }
        },
        None => {
            let names: Vec<&str> = records.iter().map(|r| r.name.as_str()).collect();
            runlog::kv("character_unknown", &format!("requested={wanted} valid_names={names:?}"));
            eprintln!("warning: unknown character \"{wanted}\"; valid names: {}", names.join(", "));
            None
        }
    };
    let (record, species, fallback) = match picked {
        Some(p) => p,
        None => {
            let r = find_record(&records, DEFAULT_CHARACTER)?;
            (r, species_of(r)?, " fallback=default")
        }
    };
    let sleeve = match defaults.get(&species, "SleeveTexture") {
        Some((Value::Object(rf), pkg)) => set.resolve(&pkg, rf),
        _ => None,
    };
    runlog::kv(
        "character",
        &format!(
            "name={} requested={} species={} sleeve_texture={} records={}{fallback}",
            record.name,
            request.unwrap_or("none"),
            species.path(),
            sleeve.as_ref().map_or("none".into(), |s| s.path()),
            records.len()
        ),
    );
    Some(Character {
        name: record.name.clone(),
        sleeve,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_upl_record() {
        let text = "[Public]\r\nPlayer=(DefaultName=\"Mr_Foster\",Race=\"DRF\",Mesh=KF_Soldier_Trip.Officeworker_Soldier,Species=KFmod.CivilianSpeciesThree,Text=\"a, b\",Sex=Male)\r\n\r\n";
        assert_eq!(
            parse_upl(text),
            vec![PlayerRecord {
                name: "Mr_Foster".into(),
                species: "KFmod.CivilianSpeciesThree".into()
            }]
        );
    }

    #[test]
    fn lower_case_species_key_and_case_insensitive_find() {
        let recs = parse_upl("Player=(DefaultName=\"Corporal_Lewis\",species=KFmod.SoldierSpecies)");
        assert_eq!(recs[0].species, "KFmod.SoldierSpecies");
        assert!(find_record(&recs, "corporal_lewis").is_some());
        assert!(find_record(&recs, "Nobody").is_none());
    }
}
