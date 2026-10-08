//! The Welder's little screen (KFMod.Welder): "Integrity:" and the weld
//! percent of the last door welded, drawn on the first-person model.
//!
//! KF: Welder.InitMaterials makes a 256 x 256 ScriptedTexture, puts it in a
//! Shader as Diffuse and SelfIllumination (so it is full bright) and sets
//! that as Skins[3]. Welder.Tick bumps the texture's Revision every tick,
//! so Welder.RenderTexture redraws it:
//!
//! - DrawTile of Texture'KillingFloorWeapons.Welder.WelderScreen' over the
//!   whole texture, tinted BackColor (128, 128, 128).
//! - With a target (not bNoTarget) and ScreenWeldPercent > 0: "Integrity:"
//!   centred at Y 50 and `ScreenWeldPercent@"%"` centred at Y 85, both in
//!   NameFont (ROFonts.ROBtsrmVr24) and NameColor R = 255 - 2p,
//!   G = 2.55p, B = 20 + p (p = the percent).
//! - Otherwise "Integrity:" at Y 50 and "-" at Y 85, white.
//!
//! The target (Welder.Tick): the LastHitActor of the fire mode firing (or
//! the last one that fired), a door set by WeldFire.Timer when a weld or
//! unweld lands. Within weaponRange x 1.5 (90 x 1.5 = 135 units) of the
//! player, the percent follows the door's WeldStrength / MaxWeld x 100;
//! farther away, or with no hit, the screen shows "-". Only a weld that
//! lands sets the target: aiming at a door does not.
//!
//! Here the screen is drawn on the CPU into an image and redrawn only when
//! what it shows changes (KF redraws every tick; the picture is the same).
//! The canvas details are native code, not in the scripts, so these are
//! assumptions: text width = sum of (glyph width + Kerning) as in the HUD;
//! glyphs are blended over the background by the font page's alpha, their
//! colour = page colour x NameColor; DrawTile's colour multiplies the
//! texture. The float printed with 2 decimals ("42.50 %") is UE2's float to
//! string ("%.2f", the format string found in the game's engine files next
//! to the vector one "%.2f,%.2f,%.2f").

use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::*;
use crate::world::door::{Doors, WeldView};

/// ScriptedScreen.SetSize(256, 256).
const SIZE: usize = 256;
/// Welder.RenderTexture's texts and their Y.
const LABEL: &str = "Integrity:";
const LABEL_Y: i32 = 50;
const VALUE_Y: i32 = 85;
/// The tile drawn as the background.
const BACKGROUND: &str = "KillingFloorWeapons.Welder.WelderScreen";
/// Skins[3]: the screen's material slot on the Welder mesh.
const SCREEN_SLOT: usize = 3;

/// A texture decoded on the CPU (RGBA8, rows top to bottom).
struct CpuTexture {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

impl CpuTexture {
    fn load(set: &PackageSet, h: &ObjectHandle) -> Option<CpuTexture> {
        let tex = ue_assets::texture::read_texture(&h.package.pkg, h.export).ok()?;
        let mip = tex.mips.first()?;
        let palette = match tex.palette_ref {
            ObjectRef::Null => None,
            rf => set.resolve(&h.package, rf).and_then(|p| ue_assets::texture::read_palette(&p.package.pkg, p.export).ok()),
        };
        let rgba = ue_assets::texture::decode_rgba(tex.format, mip, palette.as_deref())?;
        Some(CpuTexture { w: mip.width, h: mip.height, rgba })
    }

    fn texel(&self, u: usize, v: usize) -> [u8; 4] {
        let i = ((v % self.h) * self.w + (u % self.w)) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

struct ScreenFont {
    font: ue_assets::font::Font,
    pages: Vec<Option<CpuTexture>>,
}

impl ScreenFont {
    /// ScriptedTexture.TextSize: the width (assumed as Canvas.StrLen).
    fn width(&self, text: &str) -> i32 {
        text.chars().filter_map(|c| self.font.glyph(c)).map(|g| g.u_size + self.font.kerning).sum()
    }
}

/// What the screen shows: the line at Y 85 and the colour of both lines.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ScreenText {
    pub value: String,
    pub color: [u8; 4],
}

/// Welder.Tick's target check: `target` is the LastHitActor door's
/// (distance to the player, WeldStrength, MaxWeld). Within weaponRange x
/// 1.5 the screen has a target and ScreenWeldPercent follows the door;
/// otherwise bNoTarget and the percent stays as it was. Returns (bNoTarget,
/// ScreenWeldPercent).
fn tick_target(percent: f32, target: Option<(f32, f32, f32)>, weapon_range: f32) -> (bool, f32) {
    match target {
        Some((distance, weld, max)) if distance <= weapon_range * 1.5 => {
            // KF divides without a check; every weldable door has a MaxWeld.
            (false, if max > 0.0 { weld / max * 100.0 } else { percent })
        }
        _ => (true, percent),
    }
}

/// Welder.RenderTexture's choice of text and colour.
pub(super) fn screen_text(no_target: bool, percent: f32) -> ScreenText {
    if !no_target && percent > 0.0 {
        // NameColor's fields are bytes: the float is cut to a whole number.
        let byte = |f: f32| f.clamp(0.0, 255.0) as u8;
        ScreenText {
            value: format!("{percent:.2} %"),
            color: [byte(255.0 - percent * 2.0), byte(percent * 2.55), byte(20.0 + percent), 255],
        }
    } else {
        ScreenText { value: "-".into(), color: [255, 255, 255, 255] }
    }
}

#[derive(Resource)]
pub(super) struct WelderScreen {
    background: Option<CpuTexture>,
    /// Welder.BackColor.
    back_color: [u8; 4],
    font: Option<ScreenFont>,
    image: Handle<Image>,
    material: Handle<StandardMaterial>,
    /// Welder.weaponRange (90); the target counts within 1.5 x this.
    weapon_range: f32,
    /// Welder.FireModeArray: whose LastHitActor counts.
    fire_mode_array: usize,
    /// Welder.bNoTarget (true from PostBeginPlay).
    no_target: bool,
    /// Welder.ScreenWeldPercent.
    percent: f32,
    drawn: Option<ScreenText>,
    swapped: bool,
}

/// Loads what the screen needs (Welder's defaults, the background tile and
/// the font) and makes its image and material.
pub(super) fn load(
    set: &PackageSet,
    defaults: &ClassDefaults,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> WelderScreen {
    let class = crate::zeds::gore::find_class(set, "KFMod.Welder");
    let get = |p: &str| class.as_ref().and_then(|c| defaults.get(c, p));
    let back_color = match get("BackColor") {
        Some((Value::Color(c), _)) => c,
        _ => [128, 128, 128, 255],
    };
    let weapon_range = match get("weaponRange") {
        Some((Value::Float(f), _)) => f,
        _ => 90.0,
    };
    let background = set.find_object(BACKGROUND, Some("Texture")).and_then(|h| CpuTexture::load(set, &h));
    let font = match get("NameFont") {
        Some((Value::Object(rf), pkg)) => set.resolve(&pkg, rf).and_then(|h| {
            let font = ue_assets::font::read_font(&h.package.pkg, h.export).ok()?;
            let pages = font
                .textures
                .iter()
                .map(|&t| set.resolve(&h.package, t).and_then(|p| CpuTexture::load(set, &p)))
                .collect();
            runlog::kv("welder_screen_font", &format!("font={} pages={}", h.path(), font.textures.len()));
            Some(ScreenFont { font, pages })
        }),
        _ => None,
    };
    let image = images.add(Image::new(
        Extent3d { width: SIZE as u32, height: SIZE as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; SIZE * SIZE * 4],
        TextureFormat::Rgba8UnormSrgb,
        // Kept on the CPU too: redrawn in place.
        RenderAssetUsages::default(),
    ));
    // The Shader shows the texture as Diffuse and SelfIllumination: full
    // bright, opaque.
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(image.clone()),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    runlog::kv(
        "welder_screen_ready",
        &format!(
            "background={} back_color={back_color:?} font={} font_pages_loaded={} weapon_range={weapon_range}",
            background.as_ref().map_or("missing".to_string(), |b| format!("{}x{}", b.w, b.h)),
            font.is_some(),
            font.as_ref().map_or(0, |f| f.pages.iter().filter(|p| p.is_some()).count()),
        ),
    );
    WelderScreen {
        background,
        back_color,
        font,
        image,
        material,
        weapon_range,
        fire_mode_array: 0,
        no_target: true,
        percent: 0.0,
        drawn: None,
        swapped: false,
    }
}

impl WelderScreen {
    /// Welder.RenderTexture into a 256 x 256 RGBA buffer.
    fn draw(&self, text: &ScreenText, out: &mut [u8]) {
        // DrawTile(0, 0, USize, VSize, 0, 0, 256, 256, WelderScreen, BackColor).
        for y in 0..SIZE {
            for x in 0..SIZE {
                let i = (y * SIZE + x) * 4;
                let t = self.background.as_ref().map_or([0, 0, 0, 255], |b| b.texel(x * 256 / SIZE, y * 256 / SIZE));
                for c in 0..3 {
                    out[i + c] = (t[c] as u32 * self.back_color[c] as u32 / 255) as u8;
                }
                out[i + 3] = 255;
            }
        }
        let Some(font) = &self.font else { return };
        for (line, y) in [(LABEL, LABEL_Y), (text.value.as_str(), VALUE_Y)] {
            let x = (SIZE as i32 - font.width(line)) / 2;
            draw_text(font, line, x, y, text.color, out);
        }
    }
}

/// DrawText at (x, y), top left, in `color`.
fn draw_text(font: &ScreenFont, text: &str, mut x: i32, y: i32, color: [u8; 4], out: &mut [u8]) {
    for ch in text.chars() {
        let Some(g) = font.font.glyph(ch) else { continue };
        if let Some(Some(page)) = font.pages.get(g.page as usize) {
            for gy in 0..g.v_size {
                for gx in 0..g.u_size {
                    let (px, py) = (x + gx, y + gy);
                    if !(0..SIZE as i32).contains(&px) || !(0..SIZE as i32).contains(&py) {
                        continue;
                    }
                    let t = page.texel((g.start_u + gx).max(0) as usize, (g.start_v + gy).max(0) as usize);
                    let a = t[3] as u32 * color[3] as u32 / 255;
                    let i = (py as usize * SIZE + px as usize) * 4;
                    for c in 0..3 {
                        let src = t[c] as u32 * color[c] as u32 / 255;
                        out[i + c] = ((src * a + out[i + c] as u32 * (255 - a)) / 255) as u8;
                    }
                }
            }
        }
        x += g.u_size + font.font.kerning;
    }
}

/// Welder.Tick's screen part, then the redraw, and the screen material on
/// the Welder's Skins[3].
#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn update_welder_screen(
    weapons: Option<Res<Weapons>>,
    screen: Option<ResMut<WelderScreen>>,
    doors: Res<Doors>,
    view: Res<WeldView>,
    player: Query<(&Transform, Option<&crate::player::walk::Walker>), With<FlyCamera>>,
    mut images: ResMut<Assets<Image>>,
    mut part_materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut last_log: Local<Option<ScreenText>>,
) {
    let (Some(w), Some(mut s)) = (weapons, screen) else { return };
    let Some(welder) = w.defs.iter().position(|d| d.class.eq_ignore_ascii_case("KFMod.Welder")) else { return };
    // InitMaterials: Skins[3] = the screen.
    if !s.swapped {
        let def = &w.defs[welder];
        if let Some(part) = def.model.parts.iter().position(|p| p.material_index == SCREEN_SLOT)
            && let Some(&e) = def.entities.get(part)
            && let Ok(mut m) = part_materials.get_mut(e)
        {
            m.0 = s.material.clone();
            s.swapped = true;
            runlog::kv("welder_screen_skin", &format!("part={part} slot={SCREEN_SLOT}"));
        }
    }
    // FireModeArray: the mode firing, else the last one.
    if w.current == welder {
        if w.firing[0] {
            s.fire_mode_array = 0;
        } else if w.firing[1] {
            s.fire_mode_array = 1;
        }
    }
    // VSize(LastHitActor.Location - Owner.Location) <= weaponRange * 1.5.
    let Ok((cam, walker)) = player.single() else { return };
    let centre = walker.map_or(cam.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * coords::SCALE, |wk| wk.center);
    let owner = Vec3::new(-centre.z, centre.x, centre.y) / coords::SCALE;
    let target = view.last_hit[s.fire_mode_array]
        .and_then(|i| doors.doors.get(i))
        .map(|d| (Vec3::from_array(d.location()).distance(owner), d.weld, d.max_weld));
    (s.no_target, s.percent) = tick_target(s.percent, target, s.weapon_range);
    let text = screen_text(s.no_target, s.percent);
    if s.drawn.as_ref() != Some(&text) {
        if let Some(mut img) = images.get_mut(&s.image)
            && let Some(data) = img.data.as_mut()
        {
            s.draw(&text, data);
        }
        s.drawn = Some(text.clone());
    }
    // Log what the screen shows when it changes (at most the whole-percent
    // steps, to keep the log short).
    let coarse = ScreenText { value: format!("{:.0}", if s.no_target || s.percent <= 0.0 { -1.0 } else { s.percent.floor() }), color: [0; 4] };
    if last_log.as_ref() != Some(&coarse) {
        runlog::kv(
            "welder_screen",
            &format!(
                "text=\"{}\" color={:?} no_target={} percent={:.2} mode={} door={:?} held={}",
                text.value,
                text.color,
                s.no_target,
                s.percent,
                s.fire_mode_array,
                view.last_hit[s.fire_mode_array].and_then(|i| doors.doors.get(i)).map(|d| d.info.name.clone()),
                w.current == welder
            ),
        );
        *last_log = Some(coarse);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_counts_within_one_and_a_half_weapon_ranges() {
        // weaponRange 90: up to 135 units from the player.
        assert_eq!(tick_target(0.0, Some((135.0, 100.0, 400.0)), 90.0), (false, 25.0));
        // Farther: no target, the old percent kept.
        assert_eq!(tick_target(25.0, Some((136.0, 200.0, 400.0)), 90.0), (true, 25.0));
        // No door hit yet.
        assert_eq!(tick_target(0.0, None, 90.0), (true, 0.0));
    }

    #[test]
    fn screen_text_follows_welder_render_texture() {
        // No target, or nothing welded: a white "-".
        assert_eq!(screen_text(true, 50.0), ScreenText { value: "-".into(), color: [255; 4] });
        assert_eq!(screen_text(false, 0.0).value, "-");
        // R = 255 - 2p, G = 2.55p, B = 20 + p, cut to bytes.
        let t = screen_text(false, 50.0);
        assert_eq!(t.value, "50.00 %");
        assert_eq!(t.color, [155, 127, 70, 255]);
        let full = screen_text(false, 100.0);
        assert_eq!(full.value, "100.00 %");
        assert_eq!(full.color, [55, 255, 120, 255]);
        assert_eq!(screen_text(false, 12.5).value, "12.50 %");
    }
}
