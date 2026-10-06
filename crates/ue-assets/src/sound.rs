//! UE2 `Sound` and `SoundGroup` objects (KF's `.uax` packages).
//!
//! Layout worked out from the bytes (KF_9MMSnd.9mm_DryFire, 6564 bytes)
//! and checked by reading every sound in the install (`kfpkg sounds`):
//!
//! - Sound: an empty property list, FileType (a name index, "WAV"),
//!   Likelihood (float32), then a lazy array: an int32 file offset of the
//!   array's end (unused here), a compact byte count, and the bytes, which
//!   are a complete `RIFF/WAVE` file.
//! - SoundGroup: an empty property list, then the Sounds array (a compact
//!   count of object references). KF picks one of them at random.

use crate::package::{ObjectRef, Package};
use crate::properties::read_properties;
use crate::reader::{ReadErrorKind, Reader, Result};

#[derive(Clone, Debug, Default)]
pub struct Sound {
    /// "WAV" for every sound in KF.
    pub file_type: String,
    pub likelihood: f32,
    /// The embedded file, normally a `.wav`.
    pub data: Vec<u8>,
}

pub fn read_sound(pkg: &Package, index: usize) -> Result<Sound> {
    let mut r = Reader::new(pkg.export_data(index));
    read_properties(pkg, &mut r)?;
    let name = r.compact_index()?;
    if name < 0 || name as usize >= pkg.names.len() {
        return Err(r.error(ReadErrorKind::Invalid(format!("bad FileType name index {name}"))));
    }
    let file_type = pkg.name(name as usize).to_string();
    let likelihood = r.f32()?;
    let _seek_pos = r.i32()?;
    let n = r.compact_index()?;
    if n < 0 || n as usize > r.remaining() {
        return Err(r.error(ReadErrorKind::Invalid(format!("bad data size {n}"))));
    }
    let data = r.bytes(n as usize)?.to_vec();
    if r.remaining() > 0 {
        return Err(r.error(ReadErrorKind::Invalid(format!("{} bytes left over", r.remaining()))));
    }
    Ok(Sound { file_type, likelihood, data })
}

/// A SoundGroup's members (unresolved references into `pkg`).
pub fn read_sound_group(pkg: &Package, index: usize) -> Result<Vec<ObjectRef>> {
    let mut r = Reader::new(pkg.export_data(index));
    read_properties(pkg, &mut r)?;
    let n = r.compact_index()?;
    if !(0..=1024).contains(&n) {
        return Err(r.error(ReadErrorKind::Invalid(format!("bad sound count {n}"))));
    }
    let mut sounds = Vec::with_capacity(n as usize);
    for _ in 0..n {
        sounds.push(ObjectRef::from_raw(r.compact_index()?));
    }
    if r.remaining() > 0 {
        return Err(r.error(ReadErrorKind::Invalid(format!("{} bytes left over", r.remaining()))));
    }
    Ok(sounds)
}

/// Decoded PCM audio.
#[derive(Clone, Debug, Default)]
pub struct Wav {
    pub sample_rate: u32,
    pub channels: u16,
    /// Bits per sample in the file (8 or 16); `samples` are always 16-bit.
    pub bits: u16,
    /// Interleaved samples (left, right, left, ... for stereo).
    pub samples: Vec<i16>,
    /// Loop start and end in frames, from a `smpl` chunk, if the file has one.
    pub loop_points: Option<(u32, u32)>,
    /// The data chunk claimed more bytes than the file holds (cut to fit).
    pub truncated: bool,
}

impl Wav {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration(&self) -> f32 {
        self.frames() as f32 / self.sample_rate.max(1) as f32
    }
}

/// Decodes an uncompressed PCM `.wav` file (8-bit unsigned or 16-bit
/// signed, any rate, mono or stereo). Other encodings are an error: a scan
/// of the install found only PCM (format tag 1).
pub fn decode_wav(bytes: &[u8]) -> std::result::Result<Wav, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let mut fmt = None;
    let mut data: Option<(&[u8], bool)> = None;
    let mut loop_points = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32_at(pos + 4) as usize;
        let body = pos + 8;
        let end = body.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " if end - body >= 16 => {
                fmt = Some((u16_at(body), u16_at(body + 2), u32_at(body + 4), u16_at(body + 14)));
            }
            b"data" => data = Some((&bytes[body..end], body + size > bytes.len())),
            // smpl: 36 bytes of header, then loops of 24 bytes (id, type, start, end, ...).
            b"smpl" if end - body >= 36 + 24 && u32_at(body + 28) > 0 => {
                loop_points = Some((u32_at(body + 36 + 8), u32_at(body + 36 + 12)));
            }
            _ => {}
        }
        // Chunks are padded to an even size.
        pos = body.saturating_add(size).saturating_add(size & 1);
    }
    let (tag, channels, sample_rate, bits) = fmt.ok_or("no fmt chunk")?;
    let (pcm, truncated) = data.ok_or("no data chunk")?;
    if tag != 1 {
        return Err(format!("format tag {tag} is not PCM"));
    }
    if channels == 0 || sample_rate == 0 {
        return Err(format!("bad format: {channels} channels at {sample_rate} Hz"));
    }
    let samples = match bits {
        8 => pcm.iter().map(|&b| ((b as i16) - 128) << 8).collect(),
        16 => pcm.as_chunks::<2>().0.iter().map(|&c| i16::from_le_bytes(c)).collect(),
        _ => return Err(format!("{bits}-bit samples")),
    };
    Ok(Wav { sample_rate, channels, bits, samples, loop_points, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(bits: u16, channels: u16, pcm: &[u8]) -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        v.extend(16u32.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(channels.to_le_bytes());
        v.extend(22050u32.to_le_bytes());
        v.extend((22050 * channels as u32 * bits as u32 / 8).to_le_bytes());
        v.extend((channels * bits / 8).to_le_bytes());
        v.extend(bits.to_le_bytes());
        v.extend(b"data");
        v.extend((pcm.len() as u32).to_le_bytes());
        v.extend(pcm);
        v
    }

    #[test]
    fn decodes_8_and_16_bit() {
        let w = decode_wav(&wav(16, 2, &[0x00, 0x80, 0xff, 0x7f])).unwrap();
        assert_eq!((w.sample_rate, w.channels, w.frames()), (22050, 2, 1));
        assert_eq!(w.samples, vec![i16::MIN, i16::MAX]);
        let w = decode_wav(&wav(8, 1, &[0, 128, 255])).unwrap();
        assert_eq!(w.samples, vec![-32768, 0, 127 << 8]);
        assert!(!w.truncated);
    }

    #[test]
    fn rejects_non_pcm() {
        let mut v = wav(16, 1, &[0, 0]);
        v[20] = 2; // ADPCM
        assert!(decode_wav(&v).is_err());
    }
}
