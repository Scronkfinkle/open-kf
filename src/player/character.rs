//! The player's character: KF's character records (`System/*.upl`), the
//! default character, and the first-person sleeve texture its species
//! gives every weapon (KFWeapon.HandleSleeveSwapping). The record's body
//! (mesh and skins) is drawn by `player/body`.

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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerRecord {
    pub name: String,
    /// Species class path, e.g. `KFmod.SoldierSpecies`.
    pub species: String,
    /// The third-person body (SpeciesType.Setup: `LinkMesh(rec.Mesh)`;
    /// SetTeamSkin: `Skins[0] = BodySkin`, `Skins[1] = FaceSkin`), as
    /// object paths ("" when the record has none).
    pub mesh: String,
    pub body_skin: String,
    pub face_skin: String,
    /// The Karma ragdoll name (KFPawn.Setup: RagdollOverride = rec.Ragdoll).
    pub ragdoll: String,
    /// The portrait texture, e.g. `KFPortraits.gasmask_portrait` (the
    /// lobby and the perk page show it).
    pub portrait: String,
    /// xUtil.PlayerRecord.Menu: filters for the character menu
    /// (UT2k4ModelSelect leaves out records whose Menu has "DUP").
    pub menu: String,
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
        let mut rec = PlayerRecord::default();
        for (key, value) in fields(body) {
            let slot = match key.to_ascii_lowercase().as_str() {
                "defaultname" => &mut rec.name,
                "species" => &mut rec.species,
                "mesh" => &mut rec.mesh,
                "bodyskin" => &mut rec.body_skin,
                "faceskin" => &mut rec.face_skin,
                "ragdoll" => &mut rec.ragdoll,
                "portrait" => &mut rec.portrait,
                "menu" => &mut rec.menu,
                _ => continue,
            };
            *slot = value;
        }
        if !rec.name.is_empty() {
            out.push(rec);
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

/// Why `pick` fell back to the default character.
pub enum Fallback {
    Unknown { requested: String, valid: Vec<String> },
    BadSpecies { name: String, species: String },
}

/// Picks the record as KFPawn.Setup does: the requested record, else
/// (unknown name, no species, or a species that is not a
/// SPECIES_KFMaleHuman) the default character. Returns the record, its
/// species class and the reason for a fallback; None if even the default
/// is missing. Logs nothing (`choose` logs).
pub fn pick(
    set: &PackageSet,
    defaults: &ClassDefaults,
    install_root: &Path,
    request: Option<&str>,
) -> Option<(PlayerRecord, ObjectHandle, Option<Fallback>)> {
    let records = read_records(install_root);
    let species_of = |r: &PlayerRecord| {
        set.find_object(&r.species, Some("Class"))
            .filter(|c| defaults.is_a(c, "SPECIES_KFMaleHuman"))
    };
    let wanted = request.unwrap_or(DEFAULT_CHARACTER);
    let why = match find_record(&records, wanted) {
        Some(r) => match species_of(r) {
            Some(s) => return Some((r.clone(), s, None)),
            None => Fallback::BadSpecies { name: r.name.clone(), species: r.species.clone() },
        },
        None => Fallback::Unknown {
            requested: wanted.to_string(),
            valid: records.iter().map(|r| r.name.clone()).collect(),
        },
    };
    let r = find_record(&records, DEFAULT_CHARACTER)?;
    Some((r.clone(), species_of(r)?, Some(why)))
}

/// Picks the character (`pick`) and its species' sleeve texture; logs the
/// choice.
pub fn choose(set: &PackageSet, defaults: &ClassDefaults, install_root: &Path, request: Option<&str>) -> Option<Character> {
    let (record, species, why) = pick(set, defaults, install_root, request)?;
    match &why {
        Some(Fallback::Unknown { requested, valid }) => {
            runlog::kv("character_unknown", &format!("requested={requested} valid_names={valid:?}"));
            eprintln!("warning: unknown character \"{requested}\"; valid names: {}", valid.join(", "));
        }
        Some(Fallback::BadSpecies { name, species }) => {
            runlog::kv("character_bad_species", &format!("name={name} species={species}"));
        }
        None => {}
    }
    let sleeve = match defaults.get(&species, "SleeveTexture") {
        Some((Value::Object(rf), pkg)) => set.resolve(&pkg, rf),
        _ => None,
    };
    runlog::kv(
        "character",
        &format!(
            "name={} requested={} species={} sleeve_texture={} records={}{}",
            record.name,
            request.unwrap_or("none"),
            species.path(),
            sleeve.as_ref().map_or("none".into(), |s| s.path()),
            read_records(install_root).len(),
            if why.is_some() { " fallback=default" } else { "" }
        ),
    );
    Some(Character {
        name: record.name.clone(),
        sleeve,
    })
}

/// The characters KFModelSelect lists (RefreshCharacterList("DUP")):
/// every record whose Menu does not name "DUP", sorted by name (the order
/// of the native xUtil.GetPlayerList is not in the scripts: a guess).
/// The species is not checked: KFPawn.Setup
/// falls back to the default character for a bad one.
pub fn model_select_records(install_root: &Path) -> Vec<PlayerRecord> {
    read_records(install_root).into_iter().filter(|r| !r.menu.split(';').any(|m| m.trim().eq_ignore_ascii_case("DUP"))).collect()
}

/// The character asked for at runtime (the perk page's SAVE): weapons
/// swap their sleeves (weapons/weapon/load.rs).
#[derive(Message, Clone, Debug)]
pub struct ChangeCharacter(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_upl_record() {
        let text = "[Public]\r\nPlayer=(DefaultName=\"Mr_Foster\",Race=\"DRF\",Mesh=KF_Soldier_Trip.Officeworker_Soldier,Species=KFmod.CivilianSpeciesThree,BodySkin=KF_Soldier_Trip_T.Uniforms.Officeworker_cmb,FaceSkin=KF_Soldier_Trip_T.heads.Officeworker_head_diff,Ragdoll=British_Soldier1,Text=\"a, b\",Sex=Male)\r\n\r\n";
        assert_eq!(
            parse_upl(text),
            vec![PlayerRecord {
                name: "Mr_Foster".into(),
                species: "KFmod.CivilianSpeciesThree".into(),
                mesh: "KF_Soldier_Trip.Officeworker_Soldier".into(),
                body_skin: "KF_Soldier_Trip_T.Uniforms.Officeworker_cmb".into(),
                face_skin: "KF_Soldier_Trip_T.heads.Officeworker_head_diff".into(),
                ragdoll: "British_Soldier1".into(),
                portrait: String::new(),
                menu: String::new(),
            }]
        );
    }

    #[test]
    fn lower_case_species_key_and_case_insensitive_find() {
        let recs = parse_upl("Player=(DefaultName=\"Corporal_Lewis\",species=KFmod.SoldierSpecies)");
        assert_eq!(recs[0].species, "KFmod.SoldierSpecies");
        assert_eq!(recs[0].mesh, "");
        assert!(find_record(&recs, "corporal_lewis").is_some());
        assert!(find_record(&recs, "Nobody").is_none());
    }
}
