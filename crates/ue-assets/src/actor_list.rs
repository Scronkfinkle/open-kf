//! The level's actor list: which of a map's objects are actors in play.
//!
//! A map package holds every object that something still references,
//! including actors deleted in the editor (their `bDeleteMe` is set). KF
//! builds the running level from the `Level` object's actor list instead,
//! so deleted actors never exist in the game. Every map reader asks
//! [`Package::level_actor_exports`](crate::package::Package::level_actor_exports)
//! or [`Package::is_level_actor`](crate::package::Package::is_level_actor)
//! so they all see the same set. (Details in the local RE.md.)
//!
//! Saved layout of the `Level` object, after its (empty) property list:
//! - actor list: `i32` count, `i32` capacity, then `count` object
//!   references (compact index; 0 = empty slot);
//! - the level URL: Protocol, Host, Map, Portal strings, an array of option
//!   strings, Port and Valid ints;
//! - the BSP `Model` reference (compact index).

use std::collections::HashSet;

use crate::package::{ObjectRef, Package};
use crate::reader::{ReadError, Reader};

/// A map's actor list, read from its `Level` object.
#[derive(Debug, Clone)]
pub struct LevelActorList {
    /// The `Level` export (normally named `myLevel`).
    pub level: usize,
    /// Exports in the list, in list order (empty slots and imports left out).
    pub actors: Vec<usize>,
    /// Same as `actors`, for lookups.
    pub set: HashSet<usize>,
    /// Slots in the saved list, including empty ones.
    pub slots: usize,
    /// Empty (None) slots.
    pub empty: usize,
    /// Slots referring to imports (none expected).
    pub imports: usize,
    /// The level's BSP model, if the reference after the URL is a `Model` export.
    pub model: Option<usize>,
}

/// The raw decode of the bytes after the Level's property list.
#[derive(Debug, PartialEq)]
struct RawList {
    refs: Vec<ObjectRef>,
    /// None if the URL or model reference did not decode.
    model: Option<ObjectRef>,
}

fn decode(bytes: &[u8]) -> Result<RawList, ReadError> {
    let mut r = Reader::new(bytes);
    let count = r.i32()?;
    let _capacity = r.i32()?;
    // Each slot takes at least one byte.
    if count < 0 || count as usize > r.remaining() {
        return Err(r.error(crate::reader::ReadErrorKind::Invalid(format!("bad actor count {count}"))));
    }
    let refs = (0..count).map(|_| r.compact_index().map(ObjectRef::from_raw)).collect::<Result<Vec<_>, _>>()?;
    // The URL, then the BSP model. A failure here keeps the actor list.
    let model = (|| -> Result<ObjectRef, ReadError> {
        for _ in 0..4 {
            r.fstring()?;
        }
        for _ in 0..r.compact_index()?.max(0) {
            r.fstring()?;
        }
        let _port = r.i32()?;
        let _valid = r.i32()?;
        Ok(ObjectRef::from_raw(r.compact_index()?))
    })()
    .ok();
    Ok(RawList { refs, model })
}

/// Reads the actor list of the package's `Level` export. `Err` if there is
/// no `Level` (not a map) or it does not decode.
pub fn read_level_actor_list(pkg: &Package) -> Result<LevelActorList, String> {
    let level = (0..pkg.exports.len())
        .find(|&i| pkg.export_class_name(i) == "Level")
        .ok_or_else(|| "no Level export".to_string())?;
    let props = crate::properties::read_export_properties(pkg, level).map_err(|e| format!("Level properties: {e}"))?;
    let data = pkg.export_data(level);
    let raw = decode(data.get(props.end..).unwrap_or_default()).map_err(|e| format!("Level actor list: {e}"))?;
    let mut out = LevelActorList {
        level,
        actors: Vec::with_capacity(raw.refs.len()),
        set: HashSet::with_capacity(raw.refs.len()),
        slots: raw.refs.len(),
        empty: 0,
        imports: 0,
        model: None,
    };
    for rf in raw.refs {
        match rf {
            ObjectRef::Null => out.empty += 1,
            ObjectRef::Import(_) => out.imports += 1,
            ObjectRef::Export(e) if e < pkg.exports.len() => {
                if out.set.insert(e) {
                    out.actors.push(e);
                }
            }
            ObjectRef::Export(e) => return Err(format!("actor slot refers to export {e}, out of range")),
        }
    }
    if let Some(ObjectRef::Export(m)) = raw.model
        && m < pkg.exports.len()
        && pkg.export_class_name(m) == "Model"
    {
        out.model = Some(m);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fstring(out: &mut Vec<u8>, s: &str) {
        out.push(s.len() as u8 + 1);
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }

    /// Count, capacity, slots (export 0, empty, export 5, import 0), URL, model export 7.
    fn sample() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&4i32.to_le_bytes());
        b.extend_from_slice(&6i32.to_le_bytes());
        b.extend_from_slice(&[0x01, 0x00, 0x06, 0x81]);
        for s in ["unreal", "", "KF-Test.rom", ""] {
            fstring(&mut b, s);
        }
        b.push(1);
        fstring(&mut b, "Game=KFMod.KFGameType");
        b.extend_from_slice(&7777i32.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.push(0x08);
        b
    }

    #[test]
    fn decodes_slots_url_and_model() {
        let raw = decode(&sample()).unwrap();
        assert_eq!(
            raw.refs,
            vec![ObjectRef::Export(0), ObjectRef::Null, ObjectRef::Export(5), ObjectRef::Import(0)]
        );
        assert_eq!(raw.model, Some(ObjectRef::Export(7)));
    }

    #[test]
    fn keeps_list_when_url_is_cut() {
        let mut b = sample();
        b.truncate(16);
        let raw = decode(&b).unwrap();
        assert_eq!(raw.refs.len(), 4);
        assert_eq!(raw.model, None);
    }

    #[test]
    fn rejects_bad_count() {
        let mut b = sample();
        b[..4].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(decode(&b).is_err());
        b[..4].copy_from_slice(&1000i32.to_le_bytes());
        assert!(decode(&b).is_err());
    }

    /// Real data: KF-Transit's two deleted ShopVolumes are not level actors.
    /// Skipped (passes) when no Killing Floor install is found.
    #[test]
    fn transit_drops_deleted_shops() {
        let Ok(install) = crate::install::Install::discover() else {
            eprintln!("skipped: no Killing Floor install");
            return;
        };
        let Ok(pkg) = Package::open(&install.root.join("Maps").join("KF-Transit.rom")) else {
            eprintln!("skipped: KF-Transit.rom not readable");
            return;
        };
        let list = pkg.level_actor_list().unwrap();
        assert_eq!(list.actors.len(), 7515);
        assert_eq!(pkg.export_class_name(list.actors[0]), "LevelInfo");
        assert!(list.model.is_some_and(|m| pkg.export_class_name(m) == "Model"));
        let shops: Vec<&str> = pkg
            .level_actor_exports()
            .filter(|&i| pkg.export_class_name(i) == "ShopVolume")
            .map(|i| pkg.object_name(ObjectRef::Export(i)))
            .collect();
        assert_eq!(shops.len(), 3, "{shops:?}");
        assert!(!shops.contains(&"ShopVolume1") && !shops.contains(&"ShopVolume2"));
        let all = (0..pkg.exports.len()).filter(|&i| pkg.export_class_name(i) == "ShopVolume").count();
        assert_eq!(all, 5);
    }
}
