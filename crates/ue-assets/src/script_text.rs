//! UnrealScript source text stored inside `.u` packages.
//!
//! Each class owns a `TextBuffer` export named `ScriptText`. Its data is an
//! (empty) property list, two cursor positions used by the editor, and the
//! source as a string.

use crate::package::Package;
use crate::properties::read_properties;
use crate::reader::{Reader, Result};

pub struct ScriptSource {
    /// Name of the class that owns the text, e.g. `ZombieClot`.
    pub class_name: String,
    pub text: String,
}

/// Reads one `TextBuffer` export.
pub fn read_text_buffer(pkg: &Package, index: usize) -> Result<String> {
    let mut r = Reader::new(pkg.export_data(index));
    read_properties(pkg, &mut r)?;
    let _pos = r.u32()?;
    let _top = r.u32()?;
    r.fstring()
}

/// All class source texts in a package, with the indices of any that failed.
pub fn read_all(pkg: &Package) -> (Vec<ScriptSource>, Vec<(usize, String)>) {
    let mut out = Vec::new();
    let mut failed = Vec::new();
    for i in 0..pkg.exports.len() {
        let e = &pkg.exports[i];
        if pkg.export_class_name(i) != "TextBuffer" || pkg.name(e.object_name) != "ScriptText" {
            continue;
        }
        match read_text_buffer(pkg, i) {
            Ok(text) => out.push(ScriptSource {
                class_name: pkg.object_name(e.outer).to_string(),
                text,
            }),
            Err(err) => failed.push((i, err.to_string())),
        }
    }
    (out, failed)
}
