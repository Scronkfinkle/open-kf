//! UE2 `Font` objects (KF's ROFonts, ROFontsTwo, KFFonts): a glyph table
//! over texture pages.
//!
//! Layout worked out from the bytes (ROFontsTwo.ROArial14DS, 4367 bytes)
//! and checked by reading every Font in the install to its last byte
//! (`kfpkg fonts`): an empty property list; Characters (compact count,
//! then per character StartU, StartV, USize, VSize as int32 and the page
//! as a byte); Textures (compact count of object references); Kerning
//! (int32); CharRemap (compact count of (u16, u16) pairs, assumed: always
//! empty in KF); IsRemapped (int32 bool).

use crate::package::{ObjectRef, Package};
use crate::properties::read_properties;
use crate::reader::{ReadErrorKind, Reader, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontChar {
    pub start_u: i32,
    pub start_v: i32,
    pub u_size: i32,
    pub v_size: i32,
    /// Index into `Font::textures`.
    pub page: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Font {
    /// Indexed by character code.
    pub chars: Vec<FontChar>,
    pub textures: Vec<ObjectRef>,
    pub kerning: i32,
    pub remap: Vec<(u16, u16)>,
    pub is_remapped: bool,
}

impl Font {
    /// The glyph for a character, if the font has one with a size. A
    /// remapped font looks the code up in CharRemap first: (character
    /// code, glyph index), e.g. ROBtsrmVr12 maps 260 ("A" with ogonek) to
    /// glyph 256; the ASCII codes map to themselves.
    pub fn glyph(&self, c: char) -> Option<FontChar> {
        let index = if self.is_remapped {
            self.remap.iter().find(|&&(code, _)| code as u32 == c as u32).map(|&(_, i)| i as usize)?
        } else {
            c as usize
        };
        let g = *self.chars.get(index)?;
        (g.u_size > 0 && g.v_size > 0).then_some(g)
    }
}

pub fn read_font(pkg: &Package, index: usize) -> Result<Font> {
    let mut r = Reader::new(pkg.export_data(index));
    read_properties(pkg, &mut r)?;
    let n = r.compact_index()?;
    if !(0..=65536).contains(&n) {
        return Err(r.error(ReadErrorKind::Invalid(format!("bad character count {n}"))));
    }
    let mut chars = Vec::with_capacity(n as usize);
    for _ in 0..n {
        chars.push(FontChar { start_u: r.i32()?, start_v: r.i32()?, u_size: r.i32()?, v_size: r.i32()?, page: r.u8()? });
    }
    let t = r.compact_index()?;
    if !(0..=256).contains(&t) {
        return Err(r.error(ReadErrorKind::Invalid(format!("bad texture count {t}"))));
    }
    let mut textures = Vec::with_capacity(t as usize);
    for _ in 0..t {
        textures.push(ObjectRef::from_raw(r.compact_index()?));
    }
    let kerning = r.i32()?;
    let m = r.compact_index()?;
    if !(0..=65536).contains(&m) {
        return Err(r.error(ReadErrorKind::Invalid(format!("bad remap count {m}"))));
    }
    let mut remap = Vec::with_capacity(m as usize);
    for _ in 0..m {
        let a = r.u8()? as u16 | (r.u8()? as u16) << 8;
        let b = r.u8()? as u16 | (r.u8()? as u16) << 8;
        remap.push((a, b));
    }
    let is_remapped = r.i32()? != 0;
    if r.remaining() > 0 {
        return Err(r.error(ReadErrorKind::Invalid(format!("{} bytes left over", r.remaining()))));
    }
    Ok(Font { chars, textures, kerning, remap, is_remapped })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remapped_fonts_look_codes_up() {
        let mut f = Font { chars: vec![FontChar { u_size: 1, v_size: 1, ..Default::default() }; 3], ..Default::default() };
        f.chars[2].start_u = 7;
        f.is_remapped = true;
        f.remap = vec![(65, 0), (260, 2)];
        assert_eq!(f.glyph('\u{104}').map(|g| g.start_u), Some(7));
        assert!(f.glyph('B').is_none());
        f.is_remapped = false;
        assert!(f.glyph('\u{2}').is_some());
    }
}
