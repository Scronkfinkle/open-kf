//! Builds BSP lightmap pages the way KF does at load, for pages whose saved
//! copy is out of date (KF-Clandestine, KF-Forgotten, KF-Hell). Read from
//! KF's native code (DESIGN.md, "Baked lighting", L1b; details in the local
//! RE.md). Pages are 512 x 512, RGBA bytes; each surface lightmap is built
//! on its own and copied to its place on the page:
//!
//! 1. every texel starts as the zone's ambient colour x 0.5;
//! 2. per light in the lightmap's list: its shadow bits are softened with a
//!    3 x 3 filter, multiplied by the light's falloff and angle per texel
//!    (an intensity byte), and intensity x colour x 2 is added per channel,
//!    saturating at 255 (LE_Negative lights subtract).

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use crate::bsp::{BspLighting, LightBitmap, Model, SurfaceLightmap, poly_flags};
use crate::class_defaults::ClassDefaults;
use crate::level::MapLight;
use crate::package::ObjectRef;
use crate::package_set::LoadedPackage;
use crate::properties::{Value, read_export_properties};

/// Page size: KF's lightmap pages are always 512 x 512.
pub const PAGE: usize = 512;

pub const LE_STATIC_SPOT: u8 = 8;
pub const LE_SPOTLIGHT: u8 = 12;
pub const LE_NEGATIVE: u8 = 19;
pub const LE_SUNLIGHT: u8 = 20;

/// A light as the lightmap build sees it (Unreal space).
#[derive(Debug, Clone)]
pub struct BakeLight {
    pub location: [f32; 3],
    /// Where the light points (sunlight, spotlights): the actor's rotation
    /// as a unit vector.
    pub direction: [f32; 3],
    /// 25 x (LightRadius + 1).
    pub radius: f32,
    /// Light colour, 1.0 = 255 (see `light_colour`).
    pub colour: [f32; 3],
    /// LightEffect.
    pub effect: u8,
    pub cone: u8,
}

/// Unreal's FGetHSV: hue 0 red, 85 green, 170 blue; saturation 255 = white;
/// value through a brightness curve (255 gives about 0.82).
pub fn fget_hsv(hue: u8, saturation: u8, value: u8) -> [f32; 3] {
    let x = value as f32 * 0.005_490_196;
    let b = (x / (x.sqrt() + 0.01) * 0.7).clamp(0.0, 1.0);
    let h = hue as f32;
    let (k, k3) = (0.011_764_706_f32, 0.011_904_762_f32);
    let base = if hue < 86 {
        [(85.0 - h) * k, h * k, 0.0]
    } else if hue < 171 {
        [0.0, (170.0 - h) * k, (h - 85.0) * k]
    } else {
        [(h - 170.0) * k, 0.0, (255.0 - h) * k3]
    };
    let s = saturation as f32 / 255.0;
    base.map(|c| ((1.0 - c) * s + c) * b)
}

/// A float colour as KF turns it into bytes: x 255, rounded down, 0..255.
fn colour_byte(c: f32) -> u8 {
    ((c * 255.0).floor() as i32).clamp(0, 255) as u8
}

/// The starting texel colour of a zone: FGetHSV(AmbientHue,
/// AmbientSaturation, AmbientBrightness) x 0.5, as bytes.
pub fn ambient_colour(brightness: u8, hue: u8, saturation: u8) -> [u8; 3] {
    fget_hsv(hue, saturation, brightness).map(|c| colour_byte(c * 0.5))
}

/// A light's colour: FGetHSV(LightHue, LightSaturation, 255) x
/// LightBrightness / 255 x the LightType's strength x LevelInfo.Brightness.
/// LightType strength: steady, blink, flicker, texture-palette and fade-out
/// lights 1 (their value at full strength), pulse 0.6 and subtle pulse 0.9
/// (the middle of their wave), none and backdrop lights 0.
pub fn light_colour(hue: u8, saturation: u8, brightness: f32, light_type: u8, level_brightness: f32) -> [f32; 3] {
    let kind = match light_type {
        0 | 6 => 0.0,
        2 => 0.6,
        7 => 0.9,
        _ => 1.0,
    };
    fget_hsv(hue, saturation, 255).map(|c| c * brightness / 255.0 * kind * level_brightness)
}

/// Unreal rotation (65536 = a full turn) to a unit vector.
pub fn rotation_vector(pitch: i32, yaw: i32) -> [f32; 3] {
    let (p, y) = (pitch as f32 * std::f32::consts::TAU / 65536.0, yaw as f32 * std::f32::consts::TAU / 65536.0);
    [p.cos() * y.cos(), p.cos() * y.sin(), p.sin()]
}

/// Falloff table: entry i is (1 - 3q^2 + 2q^3) / q with q = sqrt((i + 1) /
/// 4096), i.e. the smooth falloff at distance q x radius divided by q (the
/// angle term below multiplies q back in).
fn falloff_table() -> &'static [f32; 4096] {
    static T: std::sync::OnceLock<Box<[f32; 4096]>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([0.0f32; 4096]);
        for (i, v) in t.iter_mut().enumerate() {
            let q = (((i + 1) as f64) * (1.0 / 4096.0)).sqrt() as f32;
            *v = (2.0 * q * q * q - 3.0 * q * q + 1.0) / q;
        }
        t
    })
}

/// The 3 x 3 shadow filter's per-row weights for three bit columns
/// (left, centre, right); rows above and below are lighter.
const FILTER: [[u32; 3]; 3] = [[24, 40, 24], [40, 64, 40], [24, 40, 24]];
const FILTER_TOTAL: u32 = 320;

/// Softened shadow (0..254) per texel of the light's box, `stride x 8`
/// texels per row; borders repeat the edge bits (a row's last byte repeats
/// its highest bit). A one-row box or a light without bits is fully lit
/// (255), as in KF.
fn smooth_shadow(b: &LightBitmap) -> (Vec<u8>, usize) {
    let rows_box = (b.max[1] - b.min[1] + 1).max(0) as usize;
    let cols_box = (b.max[0] - b.min[0] + 1).max(0) as usize;
    if b.max[1] == b.min[1] || b.bits.is_empty() {
        return (vec![255; rows_box * cols_box], cols_box);
    }
    let stride = b.stride as usize;
    let rows = b.size[1].max(0) as usize;
    let pitch = stride * 8;
    let bit = |row: usize, col: isize| -> bool {
        // Columns past the row repeat the edge: left of 0 repeats bit 0,
        // right of the last byte repeats that byte's highest bit.
        let c = col.clamp(0, pitch as isize - 1) as usize;
        b.bits.get(row * stride + c / 8).is_some_and(|byte| byte >> (c % 8) & 1 == 1)
    };
    let mut out = vec![0u8; rows * pitch];
    for out_row in 0..rows {
        for col in 0..pitch {
            let mut sum = 0u32;
            for (k, w) in FILTER.iter().enumerate() {
                // Input row above, own, below (edges repeat).
                let r = (out_row as isize + k as isize - 1).clamp(0, rows as isize - 1) as usize;
                let mut part = 0;
                for (j, weight) in w.iter().enumerate() {
                    if bit(r, col as isize + j as isize - 1) {
                        part += weight;
                    }
                }
                // Each row's share is rounded down on its own.
                sum += part * 255 / FILTER_TOTAL;
            }
            out[out_row * pitch + col] = sum as u8;
        }
    }
    (out, pitch)
}

/// Per-channel intensity-to-colour table: about min(255, 2 x i x c), in
/// KF's fixed-point form.
fn colour_table(c: f32) -> [u8; 256] {
    let ceil_log2 = |n: u32| if n <= 1 { 0 } else { 32 - (n - 1).leading_zeros() };
    let shift = 32u32.wrapping_sub(ceil_log2((c * 256.0) as i32 as u32));
    let factor = ((1u64 << (shift & 31)) as f32 * c).floor() as i64 as u32;
    let down = shift.wrapping_sub(1) & 31;
    let mut acc = 0u32;
    let mut t = [0u8; 256];
    for v in t.iter_mut() {
        *v = (acc >> down).min(255) as u8;
        acc = acc.wrapping_add(factor);
    }
    t
}

/// Light effects handled as a plain point light although KF has its own
/// formula for them (counted in `BuildStats::effects_approximated`).
fn is_exact(effect: u8) -> bool {
    matches!(effect, 0 | LE_STATIC_SPOT | LE_SPOTLIGHT | LE_NEGATIVE | LE_SUNLIGHT)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Intensity bytes (0..255) over the light's box, row by row.
fn sample(lm: &SurfaceLightmap, normal: [f32; 3], two_sided: bool, b: &LightBitmap, l: &BakeLight, shadow: &[u8], pitch: usize) -> Vec<u8> {
    let (w, h) = ((b.max[0] - b.min[0] + 1) as usize, (b.max[1] - b.min[1] + 1) as usize);
    let mut out = vec![0u8; w * h];
    if l.effect == LE_SUNLIGHT {
        let mut f = -dot(l.direction, normal);
        if two_sided {
            f = f.abs();
        } else if f <= 0.0 {
            f = 0.0;
        }
        for y in 0..h {
            for x in 0..w {
                out[y * w + x] = (shadow[y * pitch + x] as f32 * f).floor() as u8;
            }
        }
        return out;
    }
    let table = falloff_table();
    let scale = 4093.0 / (l.radius.max(1.0) * l.radius.max(1.0));
    // The light's height over the surface plane, over its radius.
    let rel = [l.location[0] - lm.base[0], l.location[1] - lm.base[1], l.location[2] - lm.base[2]];
    let height = (dot(rel, normal) / l.radius).abs();
    let spot = matches!(l.effect, LE_SPOTLIGHT | LE_STATIC_SPOT);
    let edge = 1.0 - l.cone as f32 / 256.0;
    for y in 0..h {
        for x in 0..w {
            let s = shadow[y * pitch + x];
            if s == 0 {
                continue;
            }
            let (fx, fy) = ((b.min[0] as usize + x) as f32 + 0.5, (b.min[1] as usize + y) as f32 + 0.5);
            let v: [f32; 3] = std::array::from_fn(|a| lm.base[a] + fx * lm.x_axis[a] + fy * lm.y_axis[a] - l.location[a]);
            let d2 = dot(v, v);
            let idx = (d2 * scale + 0.5).floor();
            if idx >= 4096.0 {
                continue;
            }
            let t = table[idx as usize];
            out[y * w + x] = if spot {
                let along = dot(l.direction, v);
                if along <= 0.0 || along * along <= d2 * edge * edge {
                    continue;
                }
                let f = (along / d2.sqrt() - edge) / (1.0 - edge);
                (f * f * t * s as f32 * height).floor() as u8
            } else {
                // Rounded to the nearest (KF's float-to-byte trick).
                (s as f32 * t * height).round_ties_even() as u8
            };
        }
    }
    out
}

/// Counts from building pages (logged).
#[derive(Debug, Default, Clone)]
pub struct BuildStats {
    pub lightmaps: usize,
    pub lights: usize,
    /// Lights in the lists whose actor is gone, deleted or has LightType
    /// none (not a map light), and lights whose colour is black.
    pub lights_missing: usize,
    pub lights_black: usize,
    /// Lights whose box does not fit the lightmap (should not happen).
    pub lights_bad_box: usize,
    /// LightEffect -> count of lights drawn as plain point lights although
    /// KF has its own formula for that effect.
    pub effects_approximated: BTreeMap<u8, usize>,
}

/// One surface lightmap, SizeX x SizeY RGBA bytes.
pub fn build_lightmap(
    model: &Model,
    lm: &SurfaceLightmap,
    light_of: &dyn Fn(ObjectRef) -> Option<BakeLight>,
    ambient: [u8; 3],
    stats: &mut BuildStats,
) -> Vec<u8> {
    let (w, h) = (lm.size[0].max(0) as usize, lm.size[1].max(0) as usize);
    let mut px = vec![0u8; w * h * 4];
    for p in px.as_chunks_mut::<4>().0.iter_mut() {
        *p = [ambient[0], ambient[1], ambient[2], 255];
    }
    stats.lightmaps += 1;
    let Some(surf) = model.surfs.get(lm.surf as usize) else { return px };
    let normal = model.vectors.get(surf.normal).copied().unwrap_or([0.0, 0.0, 1.0]);
    let two_sided = surf.flags & poly_flags::TWO_SIDED != 0;
    for b in &lm.lights {
        let Some(l) = light_of(b.actor) else {
            stats.lights_missing += 1;
            continue;
        };
        if l.colour.iter().all(|&c| c <= 0.0) {
            stats.lights_black += 1;
            continue;
        }
        if b.min[0] > b.max[0] || b.min[1] > b.max[1] || b.max[0] >= w as i32 || b.max[1] >= h as i32 || b.min[0] < 0 || b.min[1] < 0 {
            stats.lights_bad_box += 1;
            continue;
        }
        stats.lights += 1;
        if !is_exact(l.effect) {
            *stats.effects_approximated.entry(l.effect).or_default() += 1;
        }
        let (shadow, pitch) = smooth_shadow(b);
        let intensity = sample(lm, normal, two_sided, b, &l, &shadow, pitch);
        let tables = l.colour.map(colour_table);
        let bw = (b.max[0] - b.min[0] + 1) as usize;
        for (k, &i) in intensity.iter().enumerate() {
            let (x, y) = (b.min[0] as usize + k % bw, b.min[1] as usize + k / bw);
            let p = &mut px[(y * w + x) * 4..][..3];
            for c in 0..3 {
                let add = tables[c][i as usize];
                p[c] = if l.effect == LE_NEGATIVE { p[c].saturating_sub(add) } else { p[c].saturating_add(add) };
            }
        }
    }
    px
}

/// A whole 512 x 512 page (RGBA bytes): each of its surface lightmaps built
/// and placed at its offset. Texels no lightmap covers stay black.
pub fn build_page(
    model: &Model,
    lighting: &BspLighting,
    page: usize,
    light_of: &dyn Fn(ObjectRef) -> Option<BakeLight>,
    ambient_of: &dyn Fn(usize) -> [u8; 3],
    stats: &mut BuildStats,
) -> Vec<u8> {
    let mut out = vec![0u8; PAGE * PAGE * 4];
    let Some(t) = lighting.textures.get(page) else { return out };
    for &i in &t.lightmaps {
        let Some(lm) = lighting.surface_lightmaps.get(i as usize) else { continue };
        let px = build_lightmap(model, lm, light_of, ambient_of(lm.zone.max(0) as usize), stats);
        let w = lm.size[0].max(0) as usize;
        for (row, line) in px.chunks_exact(w * 4).enumerate() {
            let (x, y) = (lm.offset[0] as usize, lm.offset[1] as usize + row);
            if y < PAGE && x + w <= PAGE {
                out[(y * PAGE + x) * 4..][..w * 4].copy_from_slice(line);
            }
        }
    }
    out
}

/// LevelInfo.Brightness (own value, else the class default, else 1).
pub fn level_brightness(lp: &Rc<LoadedPackage>, defaults: &ClassDefaults) -> f32 {
    let pkg = &lp.pkg;
    (0..pkg.exports.len())
        .find(|&i| pkg.export_class_name(i).ends_with("LevelInfo"))
        .and_then(|i| {
            let props = read_export_properties(pkg, i).ok()?;
            match defaults.actor_value(lp, i, &props, "Brightness") {
                Some(Value::Float(f)) => Some(f),
                _ => None,
            }
        })
        .unwrap_or(1.0)
}

/// Each BSP zone's starting colour (`ambient_colour`): its ZoneInfo, or
/// the LevelInfo for zone 0 and zones without one. Own values, else class
/// defaults.
pub fn zone_ambients(lp: &Rc<LoadedPackage>, defaults: &ClassDefaults, model: &Model) -> Vec<[u8; 3]> {
    let pkg = &lp.pkg;
    let level_info = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i).ends_with("LevelInfo"));
    (0..model.num_zones.max(1))
        .map(|z| {
            let export = match model.zone_actors.get(z) {
                Some(ObjectRef::Export(e)) => Some(*e),
                _ => level_info,
            };
            let Some(e) = export else { return [0; 3] };
            let Ok(props) = read_export_properties(pkg, e) else { return [0; 3] };
            let byte = |n: &str, d: u8| match defaults.actor_value(lp, e, &props, n) {
                Some(Value::Byte(b)) => b,
                _ => d,
            };
            ambient_colour(byte("AmbientBrightness", 0), byte("AmbientHue", 0), byte("AmbientSaturation", 255))
        })
        .collect()
}

/// The map's lights for the build, by actor name (deleted lights and lights
/// with LightType none are not in `lights`, so they are skipped, as in KF).
pub fn bake_lights(lights: &[MapLight], level_brightness: f32) -> HashMap<String, BakeLight> {
    lights
        .iter()
        .map(|l| {
            (
                l.name.clone(),
                BakeLight {
                    location: l.location,
                    direction: rotation_vector(l.rotation.pitch, l.rotation.yaw),
                    radius: 25.0 * (l.radius + 1.0),
                    colour: light_colour(l.hue, l.saturation, l.brightness, l.light_type, level_brightness),
                    effect: l.effect,
                    cone: l.cone,
                },
            )
        })
        .collect()
}

/// Which texels of a page belong to a surface lightmap (for comparisons).
pub fn page_coverage(lighting: &BspLighting, page: usize) -> Vec<bool> {
    let mut out = vec![false; PAGE * PAGE];
    if let Some(t) = lighting.textures.get(page) {
        for &i in &t.lightmaps {
            let Some(lm) = lighting.surface_lightmaps.get(i as usize) else { continue };
            for y in 0..lm.size[1].max(0) as usize {
                for x in 0..lm.size[0].max(0) as usize {
                    let (px, py) = (lm.offset[0] as usize + x, lm.offset[1] as usize + y);
                    if px < PAGE && py < PAGE {
                        out[py * PAGE + px] = true;
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KF-WestLondon's saved pages are current and were made by the same
    /// build in the editor: ours must match them up to the DXT compression
    /// (about 1 of 255 on average).
    #[test]
    fn westlondon_build_matches_saved_pages() {
        let Ok(install) = crate::install::Install::discover() else {
            eprintln!("no Killing Floor install: skipped");
            return;
        };
        let set = crate::package_set::PackageSet::new(&install.root);
        let lp = set.load_path(&install.root.join("Maps/KF-WestLondon.rom")).expect("map loads");
        let defaults = ClassDefaults::new(&set);
        let contents = crate::level::read_level_with(&lp, &defaults);
        let m = contents.bsp_model.expect("BSP model");
        let model = crate::bsp::read_model(&lp.pkg, m).expect("model");
        let l = crate::bsp::read_lighting(&lp.pkg, m, &model).expect("lighting");
        let lights = bake_lights(&contents.lights, level_brightness(&lp, &defaults));
        let ambients = zone_ambients(&lp, &defaults, &model);
        let light_of = |rf: ObjectRef| lights.get(lp.pkg.object_name(rf)).cloned();
        let ambient_of = |z: usize| ambients.get(z).copied().unwrap_or([0; 3]);
        let mut stats = BuildStats::default();
        let (mut sum, mut n) = (0u64, 0u64);
        for (i, t) in l.textures.iter().enumerate() {
            let built = build_page(&model, &l, i, &light_of, &ambient_of, &mut stats);
            let mip = crate::texture::Mip { width: t.width as usize, height: t.height as usize, data: t.mips[0].clone() };
            let saved = crate::texture::decode_rgba(crate::texture::TextureFormat::from_byte(t.format), &mip, None).expect("DXT decodes");
            for (k, c) in page_coverage(&l, i).iter().enumerate() {
                if *c {
                    for ch in 0..3 {
                        sum += built[k * 4 + ch].abs_diff(saved[k * 4 + ch]) as u64;
                        n += 1;
                    }
                }
            }
        }
        let mean = sum as f64 / n as f64;
        assert!(mean < 1.5, "mean difference {mean}");
        assert_eq!(stats.lights_missing + stats.lights_bad_box, 0);
    }

    #[test]
    fn hsv_matches_known_values() {
        // White at full value: the curve gives about 0.821.
        let w = fget_hsv(0, 255, 255);
        assert!((w[0] - 0.8213).abs() < 1e-3 && w[0] == w[1] && w[1] == w[2]);
        // Value 2 at saturation 255 (white): about 0.067 grey.
        assert!((fget_hsv(0, 255, 2)[0] - 0.0669).abs() < 1e-3);
        // Pure red at saturation 0.
        let r = fget_hsv(0, 0, 255);
        assert!(r[0] > 0.8 && r[1] == 0.0 && r[2] == 0.0);
    }

    #[test]
    fn colour_table_is_twice_intensity_times_colour() {
        let t = colour_table(0.5);
        assert_eq!((t[0], t[100], t[255]), (0, 100, 255));
        let t = colour_table(1.0);
        assert_eq!((t[50], t[127], t[128], t[255]), (100, 254, 255, 255));
        let t = colour_table(0.25);
        assert_eq!((t[4], t[255]), (2, 127));
        assert!(colour_table(0.0).iter().all(|&v| v == 0));
    }

    #[test]
    fn falloff_is_one_at_the_light_and_zero_at_the_edge() {
        let t = falloff_table();
        // q x table = the smooth falloff: near 1 close by, 0 at the radius.
        assert!((t[0] * (1.0f32 / 4096.0).sqrt() - 1.0).abs() < 1e-3);
        assert!(t[4095].abs() < 1e-6);
    }

    #[test]
    fn shadow_filter_full_and_edges() {
        // 16 x 3 box, all bits set: 70 + 114 + 70 = 254 everywhere.
        let b = LightBitmap { actor: ObjectRef::Null, bits: vec![0xff; 6], size: [16, 3], stride: 2, min: [0, 0], max: [15, 2] };
        let (s, pitch) = smooth_shadow(&b);
        assert_eq!(pitch, 16);
        assert!(s.iter().all(|&v| v == 254), "{s:?}");
        // Only texel 0 of the middle row lit: its neighbour gets the side weights.
        let b = LightBitmap { bits: vec![0, 0, 1, 0, 0, 0], ..b };
        let (s, _) = smooth_shadow(&b);
        // Middle row texel 0: the left edge repeats it -> 40 + 64 of 320.
        assert_eq!(s[16], (104 * 255 / 320) as u8);
        assert_eq!(s[17], (40 * 255 / 320) as u8);
        assert_eq!(s[18], 0);
        // The rows above and below get the outer weights 24 + 40.
        assert_eq!((s[0], s[32], s[1]), ((64 * 255 / 320) as u8, (64 * 255 / 320) as u8, (24 * 255 / 320) as u8));
        // One-row boxes are fully lit.
        let b = LightBitmap { min: [0, 1], max: [3, 1], ..b };
        assert!(smooth_shadow(&b).0.iter().all(|&v| v == 255));
    }
}
