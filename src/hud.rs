//! KF's HUD (milestone 12): HUDKillingFloor's widgets drawn from the
//! game's own layout data (the class defaults), textures and digit sets.
//! H1: the bottom bar (health, armour, weight, grenades, ammo, syringe,
//! welder, medic gun charge) and the cash. See DESIGN.md, "KF's HUD".
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
const POOL: usize = 48;

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
    loaded: bool,
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
        app.init_resource::<Hud>().add_systems(PostStartup, load_hud).add_systems(PostUpdate, draw_hud);
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
    size: Vec2,
    res: Vec2,
    alpha: u8,
    quads: Vec<Quad>,
}

impl Canvas {
    fn new(size: Vec2, alpha: u8) -> Self {
        Canvas { size, res: Vec2::new(size.x / 640.0, size.y / 480.0), alpha, quads: Vec::new() }
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
    mut slots: Query<(&HudSlot, &mut Node, &mut ImageNode, &mut Visibility)>,
    mut spawned: Local<bool>,
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
    // The weight text ("1/15") needs KF's fonts: H2.
    let _ = inv.weight;
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
