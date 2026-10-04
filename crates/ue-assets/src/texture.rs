//! Texture objects: format, mipmaps, palette, and decoding to RGBA8.
//!
//! After the property list, a texture's data is an array of mipmaps (the same
//! image at halving sizes). Each mipmap is:
//! - i32: file offset just past this mip's pixel data (lets the engine skip it)
//! - compact count + that many bytes of pixel data
//! - i32 width, i32 height, u8 log2(width), u8 log2(height)

use crate::package::{ObjectRef, Package};
use crate::properties::{PropertyList, Value, read_export_properties, read_properties};
use crate::reader::{ReadError, ReadErrorKind, Reader, Result};

/// `ETextureFormat` from the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextureFormat {
    P8,
    Rgba7,
    Rgb16,
    Dxt1,
    Rgb8,
    Rgba8,
    NoData,
    Dxt3,
    Dxt5,
    L8,
    G16,
    Rrrgggbbb,
    Unknown(u8),
}

impl TextureFormat {
    pub fn from_byte(b: u8) -> Self {
        use TextureFormat::*;
        match b {
            0 => P8,
            1 => Rgba7,
            2 => Rgb16,
            3 => Dxt1,
            4 => Rgb8,
            5 => Rgba8,
            6 => NoData,
            7 => Dxt3,
            8 => Dxt5,
            9 => L8,
            10 => G16,
            11 => Rrrgggbbb,
            n => Unknown(n),
        }
    }

    /// Bytes of pixel data expected for a mip of this size, if known.
    pub fn data_size(self, w: usize, h: usize) -> Option<usize> {
        use TextureFormat::*;
        let blocks = w.div_ceil(4) * h.div_ceil(4);
        Some(match self {
            P8 | L8 => w * h,
            G16 => w * h * 2,
            Rgba8 => w * h * 4,
            Dxt1 => blocks * 8,
            Dxt3 | Dxt5 => blocks * 16,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Mip {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub format: TextureFormat,
    pub mips: Vec<Mip>,
    /// `Palette` property (only meaningful for P8).
    pub palette_ref: ObjectRef,
    /// Bytes after the mip array that we did not read. Should be 0.
    pub trailing_bytes: usize,
    pub props: PropertyList,
}

fn invalid(r: &Reader, msg: String) -> ReadError {
    r.error(ReadErrorKind::Invalid(msg))
}

/// Reads a `Texture` (or subclass) export.
pub fn read_texture(pkg: &Package, index: usize) -> Result<Texture> {
    let props = read_export_properties(pkg, index)?;
    let format = match props.get(pkg, "Format") {
        Some(Value::Byte(b)) => TextureFormat::from_byte(*b),
        // Unset properties take the class default, which is P8.
        _ => TextureFormat::P8,
    };
    let palette_ref = match props.get(pkg, "Palette") {
        Some(Value::Object(rf)) => *rf,
        _ => ObjectRef::Null,
    };

    let data = pkg.export_data(index);
    let mut r = Reader::new(data);
    r.seek(props.end)?;
    let count = r.compact_index()?;
    if !(0..=20).contains(&count) {
        return Err(invalid(&r, format!("implausible mip count {count}")));
    }
    let mut mips = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let _skip_pos = r.i32()?;
        let len = r.compact_index()?;
        if len < 0 || len as usize > r.remaining() {
            return Err(invalid(&r, format!("mip data length {len} does not fit")));
        }
        let pixels = r.bytes(len as usize)?.to_vec();
        let width = r.i32()?;
        let height = r.i32()?;
        let _ubits = r.u8()?;
        let _vbits = r.u8()?;
        if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
            return Err(invalid(&r, format!("implausible mip size {width}x{height}")));
        }
        mips.push(Mip {
            width: width as usize,
            height: height as usize,
            data: pixels,
        });
    }
    Ok(Texture {
        format,
        mips,
        palette_ref,
        trailing_bytes: r.remaining(),
        props,
    })
}

/// Reads a `Palette` export: 256 colours.
pub fn read_palette(pkg: &Package, index: usize) -> Result<Vec<[u8; 4]>> {
    let mut r = Reader::new(pkg.export_data(index));
    read_properties(pkg, &mut r)?;
    let count = r.compact_index()?;
    if !(0..=256).contains(&count) {
        return Err(invalid(&r, format!("palette has {count} colours")));
    }
    (0..count)
        .map(|_| Ok([r.u8()?, r.u8()?, r.u8()?, r.u8()?]))
        .collect()
}

/// Decodes one mip to RGBA8 (4 bytes per pixel, rows top to bottom).
/// Returns `None` for formats not handled yet or data of the wrong size.
pub fn decode_rgba(format: TextureFormat, mip: &Mip, palette: Option<&[[u8; 4]]>) -> Option<Vec<u8>> {
    let (w, h) = (mip.width, mip.height);
    if mip.data.len() < format.data_size(w, h)? {
        return None;
    }
    let src = &mip.data;
    Some(match format {
        TextureFormat::Rgba8 => {
            // Stored B, G, R, A.
            let mut out = src[..w * h * 4].to_vec();
            for px in out.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            out
        }
        TextureFormat::L8 => src[..w * h].iter().flat_map(|&l| [l, l, l, 255]).collect(),
        // 16-bit grey (terrain heightmaps); show the high byte.
        TextureFormat::G16 => src[..w * h * 2].as_chunks::<2>().0.iter().flat_map(|c| [c[1], c[1], c[1], 255]).collect(),
        TextureFormat::P8 => {
            let pal = palette?;
            src[..w * h]
                .iter()
                .flat_map(|&i| pal.get(i as usize).copied().unwrap_or([255, 0, 255, 255]))
                .collect()
        }
        TextureFormat::Dxt1 | TextureFormat::Dxt3 | TextureFormat::Dxt5 => decode_dxt(format, src, w, h),
        _ => return None,
    })
}

/// 16-bit values of a G16 mip (terrain heightmap), row by row.
pub fn g16_values(mip: &Mip) -> Option<Vec<u16>> {
    let n = mip.width * mip.height;
    if mip.data.len() < n * 2 {
        return None;
    }
    Some(
        mip.data[..n * 2]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect(),
    )
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 0x1f) as u32;
    let g = ((c >> 5) & 0x3f) as u32;
    let b = (c & 0x1f) as u32;
    [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
}

/// Decodes DXT1/3/5 (also called BC1/2/3): 4x4 pixel blocks.
fn decode_dxt(format: TextureFormat, src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let block_bytes = if format == TextureFormat::Dxt1 { 8 } else { 16 };
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    for by in 0..bh {
        for bx in 0..bw {
            let block = &src[(by * bw + bx) * block_bytes..][..block_bytes];
            let (alpha_part, color) = if format == TextureFormat::Dxt1 {
                (&block[..0], &block[..8])
            } else {
                (&block[..8], &block[8..16])
            };
            let c0 = u16::from_le_bytes([color[0], color[1]]);
            let c1 = u16::from_le_bytes([color[2], color[3]]);
            let (p0, p1) = (rgb565(c0), rgb565(c1));
            let mix = |a: u8, b: u8, wa: u32, wb: u32| ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8;
            let mut pal = [[0u8; 4]; 4];
            pal[0] = [p0[0], p0[1], p0[2], 255];
            pal[1] = [p1[0], p1[1], p1[2], 255];
            if c0 > c1 || format != TextureFormat::Dxt1 {
                for i in 0..3 {
                    pal[2][i] = mix(p0[i], p1[i], 2, 1);
                    pal[3][i] = mix(p0[i], p1[i], 1, 2);
                }
                pal[2][3] = 255;
                pal[3][3] = 255;
            } else {
                for i in 0..3 {
                    pal[2][i] = mix(p0[i], p1[i], 1, 1);
                }
                pal[2][3] = 255;
                pal[3] = [0, 0, 0, 0]; // transparent
            }
            let bits = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);

            let alphas: [u8; 16] = match format {
                TextureFormat::Dxt3 => {
                    let mut a = [0u8; 16];
                    for (i, v) in a.iter_mut().enumerate() {
                        let nib = (alpha_part[i / 2] >> ((i % 2) * 4)) & 0x0f;
                        *v = nib * 17;
                    }
                    a
                }
                TextureFormat::Dxt5 => {
                    let (a0, a1) = (alpha_part[0] as u32, alpha_part[1] as u32);
                    let mut table = [0u8; 8];
                    table[0] = a0 as u8;
                    table[1] = a1 as u8;
                    if a0 > a1 {
                        for i in 1..7 {
                            table[i + 1] = (((7 - i) as u32 * a0 + i as u32 * a1) / 7) as u8;
                        }
                    } else {
                        for i in 1..5 {
                            table[i + 1] = (((5 - i) as u32 * a0 + i as u32 * a1) / 5) as u8;
                        }
                        table[6] = 0;
                        table[7] = 255;
                    }
                    let mut abits = 0u64;
                    for (i, &b) in alpha_part[2..8].iter().enumerate() {
                        abits |= (b as u64) << (8 * i);
                    }
                    let mut a = [0u8; 16];
                    for (i, v) in a.iter_mut().enumerate() {
                        *v = table[((abits >> (3 * i)) & 7) as usize];
                    }
                    a
                }
                _ => [255; 16],
            };

            for py in 0..4 {
                for px in 0..4 {
                    let (x, y) = (bx * 4 + px, by * 4 + py);
                    if x >= w || y >= h {
                        continue;
                    }
                    let i = py * 4 + px;
                    let mut c = pal[((bits >> (2 * i)) & 3) as usize];
                    if format != TextureFormat::Dxt1 {
                        c[3] = alphas[i];
                    }
                    out[(y * w + x) * 4..][..4].copy_from_slice(&c);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dxt1_solid_block() {
        // c0 = pure red (0xF800), c1 = black, all indices 0 -> every pixel red.
        let block = [0x00, 0xF8, 0x00, 0x00, 0, 0, 0, 0];
        let mip = Mip { width: 4, height: 4, data: block.to_vec() };
        let rgba = decode_rgba(TextureFormat::Dxt1, &mip, None).unwrap();
        assert_eq!(rgba.len(), 64);
        assert!(rgba.chunks(4).all(|p| p == [255, 0, 0, 255]));
    }

    #[test]
    fn dxt1_transparent_index() {
        // c0 <= c1 selects 3-colour mode, where index 3 is transparent.
        let block = [0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let mip = Mip { width: 4, height: 4, data: block.to_vec() };
        let rgba = decode_rgba(TextureFormat::Dxt1, &mip, None).unwrap();
        assert!(rgba.chunks(4).all(|p| p[3] == 0));
    }

    #[test]
    fn dxt5_alpha_endpoints() {
        // alpha0 = 255, alpha1 = 0, all alpha indices 1 -> alpha 0 everywhere.
        let mut block = vec![255, 0];
        block.extend_from_slice(&[0b0100_1001, 0b1001_0010, 0b0010_0100, 0b0100_1001, 0b1001_0010, 0b0010_0100]);
        block.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0, 0, 0, 0]);
        let mip = Mip { width: 4, height: 4, data: block };
        let rgba = decode_rgba(TextureFormat::Dxt5, &mip, None).unwrap();
        assert!(rgba.chunks(4).all(|p| p[3] == 0 && p[0] == 255));
    }

    #[test]
    fn rgba8_is_stored_bgra() {
        let mip = Mip { width: 1, height: 1, data: vec![1, 2, 3, 4] };
        assert_eq!(decode_rgba(TextureFormat::Rgba8, &mip, None).unwrap(), vec![3, 2, 1, 4]);
    }

    #[test]
    fn g16_is_little_endian() {
        let mip = Mip { width: 2, height: 1, data: vec![0x34, 0x12, 0xff, 0x80] };
        assert_eq!(g16_values(&mip).unwrap(), vec![0x1234, 0x80ff]);
    }

    #[test]
    fn small_mips_round_up_to_blocks() {
        assert_eq!(TextureFormat::Dxt1.data_size(2, 2), Some(8));
        assert_eq!(TextureFormat::Dxt5.data_size(8, 4), Some(32));
    }
}
