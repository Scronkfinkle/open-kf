//! KF's HUD (milestone 12): HUDKillingFloor's widgets drawn from the
//! game's own layout data (the class defaults), textures and digit sets.
//! H1: the bottom bar (health, armour, weight, grenades, ammo, syringe,
//! welder, medic gun charge) and the cash. H2: KF's bitmap fonts and the
//! weight, weapon name and trader distance texts. H3: the top-right
//! circle. H4: KF's local messages (WaitingMessage, KFMainMessages)
//! through HudBase's message list. See DESIGN.md, "KF's HUD".
//!
//! DrawSpriteWidget and DrawNumericWidget are native (not in the
//! scripts). Their sizing is taken from DrawHudPassA's own weight-box
//! code: texels x TextureScale x HudCanvasScale x ResScale x HudScale,
//! with ResScaleX = width / 640, ResScaleY = height / 480, HudScale and
//! HudCanvasScale 1 (System/defuser.ini). Positions: PosX x width, PosY x
//! height. Digits sit side by side (assumed). Checked against the
//! real-game screenshots in references/.

use std::collections::HashMap;

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{LoadedPackage, ObjectHandle, PackageSet};
use ue_assets::properties::{PropertyList, Value};

use crate::map::MapRequest;
use crate::runlog;

const HUD_CLASS: &str = "KFMod.HUDKillingFloor";
/// UI image nodes reused every frame (more than the HUD ever draws).
const POOL: usize = 96;

/// IntBox: X1, Y1, X2, Y2 in texels.
#[derive(Clone, Copy, Debug, Default)]
struct IntBox([f32; 4]);

#[derive(Clone, Debug)]
struct Sprite {
    texture: Option<usize>,
    coords: IntBox,
    texture_scale: f32,
    pivot: u8,
    pos: Vec2,
    offset: Vec2,
    tint: [u8; 4],
}

#[derive(Clone, Debug)]
struct Numeric {
    min_digits: i32,
    texture_scale: f32,
    pivot: u8,
    pos: Vec2,
    offset: Vec2,
    tint: [u8; 4],
    pad_zeroes: bool,
}

#[derive(Clone, Debug, Default)]
struct DigitSet {
    texture: Option<usize>,
    /// 0-9, then the minus sign.
    coords: [IntBox; 11],
}

/// A loaded texture: its image and size in texels.
struct HudTexture {
    image: Handle<Image>,
    size: Vec2,
}

/// What a weapon shows in the ammo boxes (DrawHudPassA's class checks,
/// taken with IsA at load).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AmmoLayout {
    /// Clips and bullets in the clip (the default).
    Clips,
    /// One count (AmmoAmount(0)) and the weapon's own icon.
    Single(&'static str),
    Flamethrower,
    Shotgun,
    ZedGun,
}

#[derive(Resource, Default)]
struct Hud {
    textures: Vec<HudTexture>,
    sprites: HashMap<&'static str, Sprite>,
    numerics: HashMap<&'static str, Numeric>,
    digits_small: DigitSet,
    digits_big: DigitSet,
    /// Weapon class (lower case) -> its layout and flags.
    weapons: HashMap<String, WeaponHud>,
    /// KFHUDAlpha.
    alpha: u8,
    /// HUD.FontArrayNames and HUDKillingFloor.SmallFontArrayNames (sizes
    /// 0-8, indices into `fonts`).
    font_array: [Option<usize>; 9],
    small_font_array: [Option<usize>; 9],
    fonts: Vec<HudFont>,
    /// Hud_Bio_Clock_Circle and Hud_Bio_Circle (DrawKFHUDTextElements).
    clock_circle: Option<usize>,
    bio_circle: Option<usize>,
    /// WaitingFontArrayNames (KFFonts.KFBase02DS36, DS24).
    waiting_fonts: [Option<usize>; 2],
    loaded: bool,
}

/// The message classes we send (both CriticalEventPlus: bIsUnique,
/// bFadeMessage).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageClass {
    /// KFMod.WaitingMessage: 1 next wave inbound, 2 wave completed, 3 final
    /// wave inbound, 4 welded shut, 5 zed time, 6 door hint, 7 pickup.
    Waiting,
    /// KFMod.KFMainMessages: 0 shop boot, 1 has weapon, 2 can't carry,
    /// 3 press use to trade, 4 can't carry item.
    Main,
    /// KFMod.KFCriticalEventPlus: a map's text (ClientMessage with
    /// 'CriticalEvent', HUDKillingFloor.Message), in `text`.
    Critical,
}

/// PlayerController.ReceiveLocalizedMessage / BroadcastLocalizedMessage.
#[derive(Message, Clone, Debug)]
pub struct LocalMessage {
    pub class: MessageClass,
    pub switch: u8,
    /// The CriticalString (KFCriticalEventPlus only).
    pub text: Option<String>,
}

impl LocalMessage {
    pub fn new(class: MessageClass, switch: u8) -> Self {
        LocalMessage { class, switch, text: None }
    }
}

/// HudBase.LocalMessages: up to 8, oldest first.
#[derive(Resource, Default)]
struct LocalMessages(Vec<Shown>);

struct Shown {
    msg: LocalMessage,
    text: String,
    end_of_life: f32,
    lifetime: f32,
}

/// Per class and switch: text, lifetime, PosY, font size, colour, and
/// whether it uses the WaitingFont and RenderComplexMessage.
struct MessageStyle {
    text: String,
    lifetime: f32,
    pos_y: f32,
    font_size: i32,
    color: [u8; 3],
    waiting_font: bool,
    complex: bool,
}

/// From the classes' scripts and defaults (KFMod.int has the same
/// strings). '%Use%' is the USE key's name (KFGameType.ParseLoadingHint):
/// E here.
fn message_style(m: &LocalMessage) -> Option<MessageStyle> {
    match m.class {
        // WaitingMessage: DrawColor (255, 0, 0); PosX 0.5, DrawPivot
        // MiddleMiddle (LocalMessage); GetFontSize / GetPos / GetLifeTime
        // per switch; switches <= 3 and 5 use the WaitingFont
        // (HUDKillingFloor.LayoutMessage); bComplexString.
        MessageClass::Waiting => {
            let (text, lifetime, pos_y, font_size) = match m.switch {
                1 => ("NEXT WAVE INBOUND!", 1.0, 0.45, 4),
                2 => ("WAVE COMPLETED!|GET TO THE TRADER!", 3.0, 0.4, 4),
                3 => ("FINAL WAVE INBOUND", 1.0, 0.45, 4),
                4 => ("This door is welded shut.|Use the Welder's alt-fire to unweld.", 4.0, 0.7, 2),
                5 => ("ZED TIME ACTIVATED!", 1.5, 0.7, 2),
                6 => ("Press 'E' to open/close the door.|Use the Welder to seal closed doors.", 5.0, 0.8, 0),
                7 => ("Press 'E' to pick up Z.E.D. gun piece.", 5.0, 0.8, 0),
                _ => return None,
            };
            Some(MessageStyle { text: text.into(), lifetime, pos_y, font_size, color: [255, 0, 0], waiting_font: m.switch <= 3 || m.switch == 5, complex: true })
        }
        // KFMainMessages: DrawColor (255, 10, 10), PosY 0.8, FontSize 2,
        // Lifetime 3 (LocalMessage), plain text.
        MessageClass::Main => {
            let text = match m.switch {
                0 => "You can't stay in this shop after closing",
                1 => "You already have this weapon",
                2 => "You can not carry this weapon",
                3 => "Press 'E' to TRADE",
                4 => "You cannot carry this item",
                _ => return None,
            };
            Some(MessageStyle { text: text.into(), lifetime: 3.0, pos_y: 0.8, font_size: 2, color: [255, 10, 10], waiting_font: false, complex: false })
        }
        // KFCriticalEventPlus: Lifetime 5, DrawColor (244, 237, 205); the
        // rest LocalMessage's (PosY 0.83, FontSize 0).
        MessageClass::Critical => Some(MessageStyle {
            text: m.text.clone()?,
            lifetime: 5.0,
            pos_y: 0.83,
            font_size: 0,
            color: [244, 237, 205],
            waiting_font: false,
            complex: false,
        }),
    }
}

/// A Font with its pages loaded (indices into `Hud::textures`).
struct HudFont {
    name: String,
    font: ue_assets::font::Font,
    pages: Vec<Option<usize>>,
}

#[derive(Clone, Copy, Debug)]
struct WeaponHud {
    layout: AmmoLayout,
    syringe: bool,
    welder: bool,
    medic_gun: bool,
    husk: bool,
    /// KFWeapon.bTorchEnabled: the flashlight box.
    torch: bool,
}

/// The widgets H1 draws (all SpriteWidgets and NumericWidgets of the
/// bottom bar and the cash).
const SPRITES: [&str; 34] = [
    "HealthBG", "HealthIcon", "ArmorBG", "ArmorIcon", "WeightBG", "WeightIcon", "GrenadeBG", "GrenadeIcon", "ClipsBG", "ClipsIcon",
    "SecondaryClipsBG", "SecondaryClipsIcon", "BulletsInClipBG", "BulletsInClipIcon", "M79Icon", "PipeBombIcon", "LawRocketIcon",
    "ArrowheadIcon", "SingleBulletIcon", "FlameIcon", "FlameTankIcon", "HuskAmmoIcon", "SawAmmoIcon", "ZEDAmmoIcon", "WelderBG",
    "WelderIcon", "SyringeBG", "SyringeIcon", "MedicGunBG", "MedicGunIcon", "CashIcon", "FlashlightBG", "FlashlightIcon",
    "FlashlightOffIcon",
];
const NUMERICS: [&str; 12] = [
    "HealthDigits", "ArmorDigits", "WeightDigits", "GrenadeDigits", "ClipsDigits", "SecondaryClipsDigits", "BulletsInClipDigits",
    "WelderDigits", "SyringeDigits", "MedicGunDigits", "CashDigits", "FlashlightDigits",
];

#[derive(Component)]
struct HudSlot(usize);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hud>()
            .init_resource::<LocalMessages>()
            .add_message::<LocalMessage>()
            .add_systems(PostStartup, load_hud)
            .add_systems(PostUpdate, (receive_messages, draw_hud).chain());
    }
}

fn float_of(v: Option<&Value>) -> f32 {
    match v {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f32,
        Some(Value::Byte(b)) => *b as f32,
        _ => 0.0,
    }
}

fn byte_of(v: Option<&Value>) -> u8 {
    match v {
        Some(Value::Byte(b)) => *b,
        Some(Value::Int(i)) => *i as u8,
        _ => 0,
    }
}

fn int_box(pkg: &LoadedPackage, v: Option<&Value>) -> IntBox {
    let Some(Value::TaggedStruct { props, .. }) = v else { return IntBox::default() };
    let f = |n: &str| float_of(props.get(&pkg.pkg, n));
    IntBox([f("X1"), f("Y1"), f("X2"), f("Y2")])
}

fn tint0(pkg: &LoadedPackage, props: &PropertyList) -> [u8; 4] {
    match props.get_at(&pkg.pkg, "Tints", 0) {
        Some(Value::Color(c)) => *c,
        _ => [255, 255, 255, 255],
    }
}

struct Loader<'a> {
    set: &'a PackageSet,
    images: &'a mut Assets<Image>,
    textures: Vec<HudTexture>,
    by_path: HashMap<String, Option<usize>>,
    missing: Vec<String>,
}

impl Loader<'_> {
    /// A material by its full path ("Package.Group.Name").
    fn texture_path(&mut self, path: &str) -> Option<usize> {
        let Some(h) = self.set.find_object(path, None) else {
            self.missing.push(path.to_string());
            return None;
        };
        self.texture(&h.package, ObjectRef::Export(h.export))
    }

    /// A material reference from `pkg` -> its texture, loaded once.
    fn texture(&mut self, pkg: &std::rc::Rc<LoadedPackage>, rf: ObjectRef) -> Option<usize> {
        if rf == ObjectRef::Null {
            return None;
        }
        let from = ObjectHandle { package: pkg.clone(), export: 0 };
        let path = self.set.resolve(pkg, rf).map_or_else(|| format!("{rf:?}"), |h| h.path());
        if let Some(&i) = self.by_path.get(&path) {
            return i;
        }
        let m = ue_assets::material::resolve(self.set, &from, rf);
        let loaded = m.texture.as_ref().and_then(|t| {
            let tex = ue_assets::texture::read_texture(&t.package.pkg, t.export).ok()?;
            let mip = tex.mips.first()?;
            let size = Vec2::new(mip.width as f32, mip.height as f32);
            let image = crate::particles::decode(t, false, false, self.images)?;
            self.textures.push(HudTexture { image, size });
            Some(self.textures.len() - 1)
        });
        if loaded.is_none() {
            self.missing.push(path.clone());
        }
        self.by_path.insert(path, loaded);
        loaded
    }
}

fn load_hud(mut hud: ResMut<Hud>, request: Res<MapRequest>, mut images: ResMut<Assets<Image>>) {
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    let Some(class) = crate::gore::find_class(&set, HUD_CLASS) else {
        runlog::kv("hud_loaded", "found=false");
        return;
    };
    let mut loader = Loader { set: &set, images: &mut images, textures: Vec::new(), by_path: HashMap::new(), missing: Vec::new() };
    hud.alpha = byte_of(defaults.get(&class, "KFHUDAlpha").map(|(v, _)| v).as_ref());
    let mut absent = Vec::new();
    for name in SPRITES {
        let Some((Value::TaggedStruct { props, .. }, pkg)) = defaults.get(&class, name) else {
            absent.push(name);
            continue;
        };
        let f = |n: &str| float_of(props.get(&pkg.pkg, n));
        let texture = match props.get(&pkg.pkg, "WidgetTexture") {
            Some(Value::Object(r)) => loader.texture(&pkg, *r),
            _ => None,
        };
        let sprite = Sprite {
            texture,
            coords: int_box(&pkg, props.get(&pkg.pkg, "TextureCoords")),
            texture_scale: f("TextureScale"),
            pivot: byte_of(props.get(&pkg.pkg, "DrawPivot")),
            pos: Vec2::new(f("PosX"), f("PosY")),
            offset: Vec2::new(f("OffsetX"), f("OffsetY")),
            tint: tint0(&pkg, &props),
        };
        hud.sprites.insert(name, sprite);
    }
    for name in NUMERICS {
        let Some((Value::TaggedStruct { props, .. }, pkg)) = defaults.get(&class, name) else {
            absent.push(name);
            continue;
        };
        let f = |n: &str| float_of(props.get(&pkg.pkg, n));
        hud.numerics.insert(
            name,
            Numeric {
                min_digits: f("MinDigitCount") as i32,
                texture_scale: f("TextureScale"),
                pivot: byte_of(props.get(&pkg.pkg, "DrawPivot")),
                pos: Vec2::new(f("PosX"), f("PosY")),
                offset: Vec2::new(f("OffsetX"), f("OffsetY")),
                tint: tint0(&pkg, &props),
                pad_zeroes: f("bPadWithZeroes") != 0.0,
            },
        );
    }
    for (name, which) in [("DigitsSmall", 0), ("DigitsBig", 1)] {
        let Some((Value::TaggedStruct { props, .. }, pkg)) = defaults.get(&class, name) else {
            absent.push(name);
            continue;
        };
        let mut d = DigitSet {
            texture: match props.get(&pkg.pkg, "DigitTexture") {
                Some(Value::Object(r)) => loader.texture(&pkg, *r),
                _ => None,
            },
            ..default()
        };
        for (i, c) in d.coords.iter_mut().enumerate() {
            *c = int_box(&pkg, props.get_at(&pkg.pkg, "TextureCoords", i as u32));
        }
        if which == 0 {
            hud.digits_small = d;
        } else {
            hud.digits_big = d;
        }
    }
    // The font arrays (HUD.FontArrayNames as HUDKillingFloor sets it, and
    // its SmallFontArrayNames): each name loaded once.
    let mut font_index: HashMap<String, Option<usize>> = HashMap::new();
    let mut fonts: Vec<HudFont> = Vec::new();
    let mut font_missing = Vec::new();
    for (prop, which) in [("FontArrayNames", 0), ("SmallFontArrayNames", 1)] {
        for i in 0..9u32 {
            let Some((Value::Str(name), _)) = defaults.get_at(&class, prop, i) else { continue };
            let idx = *font_index.entry(name.to_ascii_lowercase()).or_insert_with(|| {
                let h = set.find_object(&name, Some("Font"))?;
                let font = match ue_assets::font::read_font(&h.package.pkg, h.export) {
                    Ok(f) => f,
                    Err(e) => {
                        font_missing.push(format!("{name}:{e}"));
                        return None;
                    }
                };
                let pages = font.textures.iter().map(|&t| loader.texture(&h.package, t)).collect();
                fonts.push(HudFont { name: name.clone(), font, pages });
                Some(fonts.len() - 1)
            });
            if idx.is_none() && !font_missing.iter().any(|m: &String| m.starts_with(&name)) {
                font_missing.push(name.clone());
            }
            if which == 0 {
                hud.font_array[i as usize] = idx;
            } else {
                hud.small_font_array[i as usize] = idx;
            }
        }
    }
    runlog::kv(
        "hud_fonts",
        &format!(
            "loaded=[{}] missing=[{}]",
            fonts.iter().map(|f| format!("{}:{}pages", f.name, f.pages.iter().filter(|p| p.is_some()).count())).collect::<Vec<_>>().join(" "),
            font_missing.join(" ")
        ),
    );
    hud.fonts = fonts;
    for i in 0..2u32 {
        let Some((Value::Str(name), _)) = defaults.get_at(&class, "WaitingFontArrayNames", i) else { continue };
        let Some(h) = set.find_object(&name, Some("Font")) else { continue };
        if let Ok(font) = ue_assets::font::read_font(&h.package.pkg, h.export) {
            let pages = font.textures.iter().map(|&t| loader.texture(&h.package, t)).collect();
            hud.fonts.push(HudFont { name: name.clone(), font, pages });
            hud.waiting_fonts[i as usize] = Some(hud.fonts.len() - 1);
        }
    }
    runlog::kv("hud_waiting_fonts", &format!("loaded={:?}", hud.waiting_fonts.map(|f| f.map(|i| hud.fonts[i].name.clone()))));
    hud.clock_circle = loader.texture_path("KillingFloorHUD.HUD.Hud_Bio_Clock_Circle");
    hud.bio_circle = loader.texture_path("KillingFloorHUD.HUD.Hud_Bio_Circle");
    // DrawHudPassA's weapon checks (IsA, so subclasses count).
    let is = |c: &ObjectHandle, names: &[&str]| names.iter().any(|n| defaults.is_a(c, n));
    let mut weapons = HashMap::new();
    for name in crate::weapon::BASE_WEAPONS {
        let path = format!("KFMod.{name}");
        let Some(c) = crate::gore::find_class(&set, &path) else { continue };
        let layout = if is(&c, &["LAW"]) {
            AmmoLayout::Single("LawRocketIcon")
        } else if is(&c, &["Crossbow"]) {
            AmmoLayout::Single("ArrowheadIcon")
        } else if is(&c, &["CrossBuzzSaw"]) {
            AmmoLayout::Single("SawAmmoIcon")
        } else if is(&c, &["PipeBombExplosive"]) {
            AmmoLayout::Single("PipeBombIcon")
        } else if is(&c, &["M79GrenadeLauncher", "SPGrenadeLauncher"]) {
            AmmoLayout::Single("M79Icon")
        } else if is(&c, &["HuskGun"]) {
            AmmoLayout::Single("HuskAmmoIcon")
        } else if is(&c, &["Flamethrower"]) {
            AmmoLayout::Flamethrower
        } else if is(&c, &["Shotgun", "BoomStick", "Winchester", "BenelliShotgun"]) {
            AmmoLayout::Shotgun
        } else if is(&c, &["ZEDGun"]) {
            AmmoLayout::ZedGun
        } else {
            AmmoLayout::Clips
        };
        weapons.insert(
            path.to_ascii_lowercase(),
            WeaponHud {
                layout,
                syringe: is(&c, &["Syringe"]),
                welder: is(&c, &["Welder"]),
                // Three nested ifs in DrawHudPassA (MP7M/MP5M/M7A3, then
                // + Kriss, then MP7M/MP5M only): only the MP7M and MP5M
                // get the box (KF quirk, copied).
                medic_gun: is(&c, &["MP7MMedicGun", "MP5MMedicGun"]),
                husk: is(&c, &["HuskGun"]),
                torch: matches!(defaults.get(&c, "bTorchEnabled"), Some((Value::Bool(true), _))),
            },
        );
    }
    runlog::kv(
        "hud_loaded",
        &format!(
            "sprites={} numerics={} textures={} missing_textures=[{}] absent=[{}] alpha={} weapons={} layouts=[{}]",
            hud.sprites.len(),
            hud.numerics.len(),
            loader.textures.len(),
            loader.missing.join(" "),
            absent.join(" "),
            hud.alpha,
            weapons.len(),
            {
                let mut v: Vec<String> = weapons.iter().filter(|(_, w)| w.layout != AmmoLayout::Clips).map(|(k, w)| format!("{}:{:?}", k.trim_start_matches("kfmod."), w.layout)).collect();
                v.sort();
                v.join(" ")
            }
        ),
    );
    hud.textures = loader.textures;
    hud.weapons = weapons;
    hud.loaded = true;
}

/// One textured rectangle to draw, in window pixels.
struct Quad {
    texture: usize,
    uv: Rect,
    screen: Rect,
    tint: [u8; 4],
    what: String,
}

/// EDrawPivot: the point of the box that sits at the position.
fn pivot_shift(pivot: u8, size: Vec2) -> Vec2 {
    match pivot {
        1 => Vec2::new(-size.x * 0.5, 0.0),
        2 => Vec2::new(-size.x, 0.0),
        3 => Vec2::new(-size.x, -size.y * 0.5),
        4 => -size,
        5 => Vec2::new(-size.x * 0.5, -size.y),
        6 => Vec2::new(0.0, -size.y),
        7 => Vec2::new(0.0, -size.y * 0.5),
        8 => -size * 0.5,
        _ => Vec2::ZERO,
    }
}

struct Canvas {
    /// Window size in logical pixels (what the UI nodes use).
    size: Vec2,
    res: Vec2,
    alpha: u8,
    /// Physical pixels per logical pixel. KF's fonts are fixed pixel sizes
    /// on its canvas (physical pixels): text is laid out in physical
    /// pixels and divided by this.
    scale_factor: f32,
    quads: Vec<Quad>,
}

impl Canvas {
    fn new(size: Vec2, alpha: u8) -> Self {
        Canvas { size, res: Vec2::new(size.x / 640.0, size.y / 480.0), alpha, scale_factor: 1.0, quads: Vec::new() }
    }

    /// The canvas width KF's font choices look at (C.ClipX).
    fn clip_x(&self) -> f32 {
        self.size.x * self.scale_factor
    }

    /// Canvas.StrLen: the text's size in physical pixels (glyph sizes plus
    /// Kerning, times the font scale; native, assumed).
    fn text_size(font: &HudFont, text: &str, scale: f32) -> Vec2 {
        let mut w = 0.0f32;
        let mut h = 0.0f32;
        for ch in text.chars() {
            if let Some(g) = font.font.glyph(ch) {
                w += (g.u_size + font.font.kerning) as f32 * scale;
                h = h.max(g.v_size as f32 * scale);
            }
        }
        Vec2::new(w, h)
    }

    /// Canvas.DrawText at a physical pixel position (SetPos), tinted.
    fn text(&mut self, font: &HudFont, text: &str, at: Vec2, scale: f32, tint: [u8; 4], what: &str) {
        let mut x = at.x;
        for ch in text.chars() {
            let Some(g) = font.font.glyph(ch) else { continue };
            let size = Vec2::new(g.u_size as f32, g.v_size as f32) * scale;
            if let Some(Some(page)) = font.pages.get(g.page as usize) {
                let min = Vec2::new(x, at.y) / self.scale_factor;
                self.quads.push(Quad {
                    texture: *page,
                    uv: Rect::new(g.start_u as f32, g.start_v as f32, (g.start_u + g.u_size) as f32, (g.start_v + g.v_size) as f32),
                    screen: Rect::from_corners(min, min + size / self.scale_factor),
                    tint,
                    what: format!("{what}'{ch}'"),
                });
            }
            x += (g.u_size + font.font.kerning) as f32 * scale;
        }
    }

    /// DrawSpriteWidget (assumed native rules, see the module comment).
    fn sprite(&mut self, s: &Sprite, what: &str) {
        let Some(texture) = s.texture else { return };
        let c = s.coords.0;
        let size = Vec2::new((c[2] - c[0]).abs(), (c[3] - c[1]).abs()) * s.texture_scale * self.res;
        let at = s.pos * self.size + s.offset * s.texture_scale * self.res + pivot_shift(s.pivot, size);
        self.quads.push(Quad {
            texture,
            uv: Rect::new(c[0], c[1], c[2], c[3]),
            screen: Rect::from_corners(at, at + size),
            tint: [s.tint[0], s.tint[1], s.tint[2], self.alpha],
            what: what.to_string(),
        });
    }

    /// DrawNumericWidget: the digits side by side from the digit set.
    fn numeric(&mut self, n: &Numeric, value: i32, digits: &DigitSet, tint: Option<[u8; 3]>, what: &str) {
        let Some(texture) = digits.texture else { return };
        let mut text = value.unsigned_abs().to_string();
        let width = n.min_digits.max(0) as usize;
        if text.len() < width {
            let pad = if n.pad_zeroes { "0" } else { "" };
            text = pad.repeat(width - text.len()) + &text;
        }
        let mut idx: Vec<usize> = text.bytes().map(|b| (b - b'0') as usize).collect();
        if value < 0 {
            idx.insert(0, 10);
        }
        let sizes: Vec<Vec2> = idx
            .iter()
            .map(|&i| {
                let c = digits.coords[i].0;
                Vec2::new(c[2] - c[0], c[3] - c[1]) * n.texture_scale * self.res
            })
            .collect();
        let total = Vec2::new(sizes.iter().map(|s| s.x).sum(), sizes.iter().map(|s| s.y).fold(0.0, f32::max));
        let mut at = n.pos * self.size + n.offset * n.texture_scale * self.res + pivot_shift(n.pivot, total);
        let rgb = tint.unwrap_or([n.tint[0], n.tint[1], n.tint[2]]);
        for (k, &i) in idx.iter().enumerate() {
            let c = digits.coords[i].0;
            self.quads.push(Quad {
                texture,
                uv: Rect::new(c[0], c[1], c[2], c[3]),
                screen: Rect::from_corners(at, at + sizes[k]),
                tint: [rgb[0], rgb[1], rgb[2], self.alpha],
                what: format!("{what}[{k}]"),
            });
            at.x += sizes[k].x;
        }
    }
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn draw_hud(
    mut commands: Commands,
    hud: Res<Hud>,
    window: Query<&Window>,
    health: Res<crate::combat::PlayerHealth>,
    armour: Res<crate::armour::Armour>,
    ammo: Res<crate::combat::AmmoDisplay>,
    (dosh, inv, time): (Res<crate::dosh::Dosh>, Res<crate::buy_menu::ShopInventory>, Res<Time>),
    (script, frames): (Res<crate::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    (shops, menu, player, game, options): (Res<crate::trader::Shops>, Res<crate::buy_menu::BuyMenu>, PlayerQuery, Res<crate::game::WaveGame>, Res<crate::game::GameOptions>),
    mut slots: Query<(&HudSlot, &mut Node, &mut ImageNode, &mut Visibility)>,
    mut spawned: Local<bool>,
    mut messages: ResMut<LocalMessages>,
) {
    if !hud.loaded {
        return;
    }
    if !*spawned {
        *spawned = true;
        for i in 0..POOL {
            commands.spawn((
                Node { position_type: PositionType::Absolute, ..default() },
                // KF stretches each picture to its box.
                ImageNode { image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() },
                Visibility::Hidden,
                GlobalZIndex(10 + i as i32),
                HudSlot(i),
            ));
        }
        return;
    }
    let Ok(win) = window.single() else { return };
    let mut c = Canvas::new(Vec2::new(win.width(), win.height()), hud.alpha);
    c.scale_factor = win.scale_factor();
    let sprite = |c: &mut Canvas, name: &str| {
        if let Some(s) = hud.sprites.get(name) {
            c.sprite(s, name);
        }
    };
    let numeric = |c: &mut Canvas, name: &str, value: i32, big: bool, tint: Option<[u8; 3]>| {
        if let Some(n) = hud.numerics.get(name) {
            c.numeric(n, value, if big { &hud.digits_big } else { &hud.digits_small }, tint, name);
        }
    };
    // UpdateHud's health colour: under 50 it flashes yellow and red
    // (SwitchDigitColorTime, 0.2 s each); else (255, 50, 50). The bile
    // colour (VomitHudTimer) is not done.
    let hp = health.health.max(0.0) as i32;
    let hp_tint = if hp < 50 {
        if (time.elapsed_secs() / 0.2) as i64 % 2 == 0 { [255, 200, 0] } else { [255, 0, 0] }
    } else {
        [255, 50, 50]
    };
    sprite(&mut c, "HealthBG");
    sprite(&mut c, "HealthIcon");
    numeric(&mut c, "HealthDigits", hp, false, Some(hp_tint));
    sprite(&mut c, "ArmorBG");
    sprite(&mut c, "ArmorIcon");
    numeric(&mut c, "ArmorDigits", armour.strength as i32, false, None);
    // The weight box: DrawTile of the whole texture, 1.5 times as wide.
    if let Some(s) = hud.sprites.get("WeightBG")
        && let Some(t) = s.texture
    {
        let size = hud.textures[t].size;
        let at = s.pos * c.size;
        let screen = Rect::from_corners(at, at + size * s.texture_scale * Vec2::new(1.5, 1.0) * c.res);
        c.quads.push(Quad { texture: t, uv: Rect::from_corners(Vec2::ZERO, size), screen, tint: [s.tint[0], s.tint[1], s.tint[2], c.alpha], what: "WeightBG".into() });
    }
    sprite(&mut c, "WeightIcon");
    // "1/15": LoadSmallFontStatic(5), scaled ClipX / 1024, at WeightDigits'
    // position in its colour (alpha KFHUDAlpha).
    if let (Some(f), Some(n)) = (hud.small_font_array[5].map(|i| &hud.fonts[i]), hud.numerics.get("WeightDigits")) {
        let text = format!("{}/{}", inv.weight as i32, crate::buy_menu::MAX_CARRY_WEIGHT as i32);
        let at = n.pos * c.size * c.scale_factor;
        let scale = c.clip_x() / 1024.0;
        c.text(f, &text, at, scale, [n.tint[0], n.tint[1], n.tint[2], hud.alpha], "Weight");
    }
    sprite(&mut c, "GrenadeBG");
    sprite(&mut c, "GrenadeIcon");
    numeric(&mut c, "GrenadeDigits", ammo.frags.unwrap_or(0) as i32, false, None);
    let w = hud.weapons.get(&ammo.class.to_ascii_lowercase()).copied();
    if let Some(w) = w {
        if w.syringe {
            sprite(&mut c, "SyringeBG");
            sprite(&mut c, "SyringeIcon");
            let v = ammo.syringe.unwrap_or(0) as i32;
            numeric(&mut c, "SyringeDigits", v, false, Some(charge_tint(v)));
        } else {
            if w.medic_gun {
                let v = ammo.heal.unwrap_or(0) as i32;
                sprite(&mut c, "MedicGunBG");
                sprite(&mut c, "MedicGunIcon");
                numeric(&mut c, "MedicGunDigits", v, false, Some(charge_tint(v)));
            }
            if w.welder {
                sprite(&mut c, "WelderBG");
                sprite(&mut c, "WelderIcon");
                numeric(&mut c, "WelderDigits", ammo.weld_percent.unwrap_or(0) as i32, false, None);
            } else if let Some((mag, spare)) = ammo.ammo {
                // CalculateAmmo: magazines left besides the one loaded, a
                // part one counting; bHoldToReload weapons show rounds.
                let clips = if ammo.hold_to_reload {
                    spare as i32
                } else {
                    let cap = ammo.capacity.max(1);
                    (spare / cap + u32::from(spare % cap > 0)) as i32
                };
                sprite(&mut c, "ClipsBG");
                let clips_value = if matches!(w.layout, AmmoLayout::Single(_)) { (mag + spare) as i32 } else { clips };
                if w.husk {
                    // DrawHudPassA moves the Husk Gun's digits to 0.873.
                    if let Some(n) = hud.numerics.get("ClipsDigits") {
                        let moved = Numeric { pos: Vec2::new(0.873, n.pos.y), ..n.clone() };
                        c.numeric(&moved, clips_value, &hud.digits_small, None, "ClipsDigits");
                    }
                } else {
                    numeric(&mut c, "ClipsDigits", clips_value, false, None);
                }
                match w.layout {
                    AmmoLayout::Single(icon) => sprite(&mut c, icon),
                    other => {
                        sprite(&mut c, "BulletsInClipBG");
                        numeric(&mut c, "BulletsInClipDigits", mag as i32, false, None);
                        match other {
                            AmmoLayout::Flamethrower => {
                                sprite(&mut c, "FlameIcon");
                                sprite(&mut c, "FlameTankIcon");
                            }
                            AmmoLayout::Shotgun => {
                                sprite(&mut c, "SingleBulletIcon");
                                sprite(&mut c, "BulletsInClipIcon");
                            }
                            AmmoLayout::ZedGun => {
                                sprite(&mut c, "ClipsIcon");
                                sprite(&mut c, "ZEDAmmoIcon");
                            }
                            _ => {
                                sprite(&mut c, "ClipsIcon");
                                sprite(&mut c, "BulletsInClipIcon");
                            }
                        }
                    }
                }
                // The flashlight box (bTorchEnabled). We have no flashlight
                // yet: the battery shows full and the light off.
                if w.torch {
                    sprite(&mut c, "FlashlightBG");
                    numeric(&mut c, "FlashlightDigits", 100, false, None);
                    sprite(&mut c, "FlashlightOffIcon");
                }
            }
            // Secondary ammo (bHasSecondaryAmmo: the M4 203's grenades).
            if let Some(n) = ammo.alt_ammo {
                sprite(&mut c, "SecondaryClipsBG");
                numeric(&mut c, "SecondaryClipsDigits", n as i32, false, None);
                sprite(&mut c, "SecondaryClipsIcon");
            }
        }
    }
    sprite(&mut c, "CashIcon");
    numeric(&mut c, "CashDigits", dosh.score as i32, true, None);
    // DrawWeaponName: GetFontSizeIndex(C, -1), (255, 50, 50, KFHUDAlpha),
    // right edge at 0.983 x ClipX, top at 0.90 x ClipY.
    if !ammo.weapon.is_empty()
        && let Some(f) = hud.font_array[font_size_index(c.clip_x(), -1)].map(|i| &hud.fonts[i])
    {
        let size = Canvas::text_size(f, ammo.weapon, 1.0);
        let phys = c.size * c.scale_factor;
        c.text(f, ammo.weapon, Vec2::new(phys.x * 0.983 - size.x, phys.y * 0.90), 1.0, [255, 50, 50, hud.alpha], "WeaponName");
    }
    // DrawKFHUDTextElements: the top-right circle (not while shopping;
    // wave mode only, standing in for bMatchHasBegun).
    if !menu.open && options.mode == crate::game::GameMode::Waves {
        top_right_circle(&mut c, &hud, &game);
    }
    // DrawTraderDistance (from DrawKFHUDTextElements: not while shopping,
    // only with a current shop): "Trader: Nm", N = int(distance / 50),
    // centred on SizeX / 14, top at SizeX / 10, (255, 50, 50, 255).
    if !menu.open
        && let Some(cur) = shops.current
        && let Ok((cam, walker)) = player.single()
    {
        let centre = walker.map_or(cam.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * crate::coords::SCALE, |w| w.center);
        let pawn = Vec3::new(-centre.z, centre.x, centre.y) / crate::coords::SCALE;
        let text = format!("Trader: {}m", ((shops.shops[cur].location - pawn).length() / 50.0) as i32);
        let clip = c.clip_x();
        let size_index = if clip <= 640.0 {
            7
        } else if clip <= 800.0 {
            6
        } else if clip <= 1024.0 {
            5
        } else if clip <= 1280.0 {
            4
        } else {
            3
        };
        if let Some(f) = hud.font_array[size_index].map(|i| &hud.fonts[i]) {
            let w = Canvas::text_size(f, &text, 1.0).x;
            c.text(f, &text, Vec2::new(clip / 14.0 - w / 2.0, clip / 10.0), 1.0, [255, 50, 50, 255], "Trader");
        }
    }

    // DisplayLocalMessages (called before DrawWeaponName in DrawHUD; drawn
    // last here, on top).
    display_local_messages(&mut c, &hud, &mut messages, time.elapsed_secs());

    if script.0.iter().any(|(f, a)| *f == frames.0 && a == "hud_dump") {
        let lines: Vec<String> = c
            .quads
            .iter()
            .map(|q| format!("{}:({:.0},{:.0})-({:.0},{:.0})", q.what, q.screen.min.x, q.screen.min.y, q.screen.max.x, q.screen.max.y))
            .collect();
        runlog::kv("hud_dump", &format!("window={:.0}x{:.0} weapon={} quads={} {}", c.size.x, c.size.y, ammo.class, c.quads.len(), lines.join(" ")));
    }
    for (slot, mut node, mut image, mut vis) in &mut slots {
        let Some(q) = c.quads.get(slot.0) else {
            *vis = Visibility::Hidden;
            continue;
        };
        node.left = Val::Px(q.screen.min.x);
        node.top = Val::Px(q.screen.min.y);
        node.width = Val::Px(q.screen.width());
        node.height = Val::Px(q.screen.height());
        let t = &hud.textures[q.texture];
        if image.image != t.image {
            image.image = t.image.clone();
        }
        image.rect = Some(q.uv);
        image.color = Color::srgba_u8(q.tint[0], q.tint[1], q.tint[2], q.tint[3]);
        *vis = Visibility::Inherited;
    }
}

/// The circle: CircleSize = Min(128 x SizeX / 1024, 128), drawn white at
/// (ClipX - CircleSize, 2); fonts scaled Min(SizeX / 1024, 1). Between
/// waves the clock (LoadFont(2), "mm:ss" of TimeToNextWave, centred);
/// in a wave the biohazard sign with GRI.MaxMonsters (LoadFont(1), raised
/// by YL / 1.5) and "Wave N/F" (LoadFont(5), lowered by YL / 2.5).
fn top_right_circle(c: &mut Canvas, hud: &Hud, game: &crate::game::WaveGame) {
    use crate::game::Phase;
    let clip = c.clip_x();
    let res = clip / 1024.0;
    let circle = (128.0 * res).min(128.0);
    let font_scale = res.min(1.0);
    let in_wave = matches!(game.phase, Phase::Wave | Phase::BossWave);
    let tex = if in_wave { hud.bio_circle } else { hud.clock_circle };
    if let Some(t) = tex {
        let min = Vec2::new(clip - circle, 2.0) / c.scale_factor;
        let size = hud.textures[t].size;
        c.quads.push(Quad {
            texture: t,
            uv: Rect::from_corners(Vec2::ZERO, size.min(Vec2::splat(256.0))),
            screen: Rect::from_corners(min, min + Vec2::splat(circle) / c.scale_factor),
            tint: [255, 255, 255, 255],
            what: "Circle".into(),
        });
    }
    let centre_x = clip - circle / 2.0;
    let tint = [255, 50, 50, hud.alpha];
    let font = |i: usize| hud.font_array[i].map(|f| &hud.fonts[f]);
    if !in_wave {
        let t = game.countdown.max(0);
        let (m, sec) = (t / 60, t % 60);
        let text = format!("{m:02}:{sec:02}");
        if let Some(f) = font(2) {
            let sz = Canvas::text_size(f, &text, font_scale);
            c.text(f, &text, Vec2::new(centre_x - sz.x / 2.0, circle / 2.0 - sz.y / 2.0), font_scale, tint, "Clock");
        }
    } else {
        // GRI.MaxMonsters: TotalMaxMonsters at SetupWave, then TotalMaxMonsters
        // + NumMonsters - 1 at each kill; spawning keeps that sum, so ours is
        // the zeds still to come plus those alive. (In the boss wave KF only
        // counts his helpers after the next kill; ours counts them at once.)
        let left = (game.total_max_monsters.max(0) as usize + game.living).to_string();
        if let Some(f) = font(1) {
            let sz = Canvas::text_size(f, &left, font_scale);
            c.text(f, &left, Vec2::new(centre_x - sz.x / 2.0, circle / 2.0 - sz.y / 1.5), font_scale, tint, "ZedsLeft");
        }
        // WaveString @ (WaveNumber + 1) $ "/" $ FinalWave: in the boss wave
        // WaveNum is FinalWave, so KF shows e.g. "Wave 5/4" (copied).
        let text = format!("Wave {}/{}", game.wave_num + 1, game.final_wave);
        if let Some(f) = font(5) {
            let sz = Canvas::text_size(f, &text, font_scale);
            c.text(f, &text, Vec2::new(centre_x - sz.x / 2.0, circle / 2.0 + sz.y / 2.5), font_scale, tint, "Wave");
        }
    }
}

/// HudBase.LocalizedMessage for our (unique) classes: replaces the
/// message of the same class, else takes a free slot, else drops the
/// oldest.
fn receive_messages(time: Res<Time>, mut incoming: MessageReader<LocalMessage>, mut list: ResMut<LocalMessages>) {
    let now = time.elapsed_secs();
    for m in incoming.read() {
        let Some(style) = message_style(m) else { continue };
        let shown = Shown { msg: m.clone(), text: style.text.clone(), end_of_life: now + style.lifetime, lifetime: style.lifetime };
        if let Some(i) = list.0.iter().position(|s| s.msg.class == m.class) {
            list.0[i] = shown;
        } else {
            if list.0.len() == 8 {
                list.0.remove(0);
            }
            list.0.push(shown);
        }
        runlog::kv("hud_message", &format!("class={:?} switch={} text=\"{}\"", m.class, m.switch, style.text));
    }
}

/// DisplayLocalMessages: dead ones culled (bFadeMessage), each drawn
/// faded by its remaining life. Messages at the same PosY stack down
/// (SM_Down); not met by ours in practice.
fn display_local_messages(c: &mut Canvas, hud: &Hud, list: &mut LocalMessages, now: f32) {
    list.0.retain(|s| s.end_of_life - now > 0.0);
    let clip = c.clip_x();
    let phys = c.size * c.scale_factor;
    let mut stack: Vec<(f32, f32)> = Vec::new();
    for s in &list.0 {
        let Some(style) = message_style(&s.msg) else { continue };
        // LayoutMessage: the font.
        let font = if style.waiting_font {
            hud.waiting_fonts[if clip <= 1024.0 { 1 } else { 0 }]
        } else {
            hud.font_array[font_size_index(clip, style.font_size)]
        };
        let Some(font) = font.map(|i| &hud.fonts[i]) else { continue };
        let alpha = (255.0 * ((s.end_of_life - now) / s.lifetime).clamp(0.0, 1.0)) as u8;
        let tint = [style.color[0], style.color[1], style.color[2], alpha];
        // LayoutMessage's TextSize: the whole string at scale 1 (Canvas
        // reset), '|' and all; GetScreenCoords centres that box on (PosX,
        // PosY) (DP_MiddleMiddle).
        let whole = Canvas::text_size(font, &s.text, 1.0);
        let mut pos_y = style.pos_y;
        for &(y, dy) in &stack {
            if y == style.pos_y {
                pos_y += dy;
            }
        }
        let top = pos_y * phys.y - whole.y * 0.5;
        if style.complex {
            // WaitingMessage.RenderComplexMessage: scale ClipX / 1024, each
            // line centred on ClipX / 2, the second YL below the first.
            let scale = clip / 1024.0;
            let (first, second) = match s.text.split_once('|') {
                Some((a, b)) => (a, Some(b)),
                None => (s.text.as_str(), None),
            };
            let sz = Canvas::text_size(font, first, scale);
            c.text(font, first, Vec2::new(clip / 2.0 - sz.x / 2.0, top), scale, tint, "Message");
            if let Some(second) = second {
                let sz = Canvas::text_size(font, second, scale);
                c.text(font, second, Vec2::new(clip / 2.0 - sz.x / 2.0, top + sz.y), scale, tint, "Message");
            }
        } else {
            c.text(font, &s.text, Vec2::new(0.5 * phys.x - whole.x * 0.5, top), 1.0, tint, "Message");
        }
        stack.push((style.pos_y, whole.y / phys.y));
    }
}

/// HUD.GetFontSizeIndex: one size step per width threshold passed, then
/// LoadFont(Clamp(8 - FontSize, 0, 8)).
fn font_size_index(clip_x: f32, font_size: i32) -> usize {
    let steps = [512.0, 640.0, 800.0, 1024.0, 1280.0, 1600.0].iter().filter(|&&t| clip_x >= t).count() as i32;
    (8 - (font_size + steps)).clamp(0, 8) as usize
}

type PlayerQuery<'w, 's> = Query<'w, 's, (&'static Transform, Option<&'static crate::walk::Walker>), With<crate::camera::FlyCamera>>;

/// UpdateHud's syringe / medic gun digit colours by charge.
fn charge_tint(v: i32) -> [u8; 3] {
    if v < 50 {
        [128, 128, 128]
    } else if v < 100 {
        [192, 96, 96]
    } else {
        [255, 64, 64]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_box_lands_where_kf_draws_it() {
        // HealthBG at 1280 x 960: PosX 0.015, 128 x 64 texels x 0.35 x
        // (2, 2) = 89.6 x 44.8 at (19.2, 897.6).
        let mut c = Canvas::new(Vec2::new(1280.0, 960.0), 200);
        let s = Sprite {
            texture: Some(0),
            coords: IntBox([0.0, 0.0, 128.0, 64.0]),
            texture_scale: 0.35,
            pivot: 0,
            pos: Vec2::new(0.015, 0.935),
            offset: Vec2::ZERO,
            tint: [255; 4],
        };
        c.sprite(&s, "HealthBG");
        let r = c.quads[0].screen;
        assert!((r.min.x - 19.2).abs() < 0.01 && (r.min.y - 897.6).abs() < 0.01);
        assert!((r.width() - 89.6).abs() < 0.01 && (r.height() - 44.8).abs() < 0.01);
    }

    #[test]
    fn message_styles_follow_the_classes() {
        let w = |n| message_style(&LocalMessage::new(MessageClass::Waiting, n)).unwrap();
        // Wave messages: WaitingFont, 1 s at 0.45; wave completed 3 s at 0.4.
        assert!(w(1).waiting_font && w(1).lifetime == 1.0 && w(1).pos_y == 0.45);
        assert!(w(2).text.contains('|') && w(2).lifetime == 3.0 && w(2).pos_y == 0.4);
        // Welded shut: Arial size 2, 4 s at 0.7.
        assert!(!w(4).waiting_font && w(4).font_size == 2 && w(4).pos_y == 0.7);
        let m = message_style(&LocalMessage::new(MessageClass::Main, 3)).unwrap();
        assert_eq!((m.text.as_str(), m.lifetime, m.pos_y), ("Press 'E' to TRADE", 3.0, 0.8));
        // A critical message needs its text.
        assert!(message_style(&LocalMessage::new(MessageClass::Critical, 0)).is_none());
    }

    #[test]
    fn weapon_name_font_at_1280_is_size_4() {
        assert_eq!(font_size_index(1280.0, -1), 4);
        assert_eq!(font_size_index(1024.0, -1), 5);
        assert_eq!(font_size_index(2556.0, -1), 3);
    }

    #[test]
    fn digits_sit_side_by_side_with_minus_first() {
        let mut d = DigitSet { texture: Some(0), ..default() };
        for (i, b) in d.coords.iter_mut().enumerate() {
            *b = IntBox([i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, 20.0]);
        }
        let n = Numeric { min_digits: 0, texture_scale: 1.0, pivot: 0, pos: Vec2::ZERO, offset: Vec2::ZERO, tint: [255; 4], pad_zeroes: false };
        let mut c = Canvas::new(Vec2::new(640.0, 480.0), 255);
        c.numeric(&n, -12, &d, None, "n");
        let xs: Vec<f32> = c.quads.iter().map(|q| q.screen.min.x).collect();
        assert_eq!(xs, vec![0.0, 10.0, 20.0]);
        assert_eq!(c.quads[0].uv.min.x, 100.0);
    }
}
