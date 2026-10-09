//! The loading screen shown while a map loads: KF's LoadingClass,
//! ROInterface.ROServerLoading ("Deploying to <map>" over a random
//! picture). See DESIGN.md, "Loading screen".
//!
//! What KF draws (ROServerLoading and its parents UT2K4ServerLoading,
//! UT2K4LoadingPageBase; positions are fractions of the screen):
//! - a background picked at random from `Backgrounds`
//!   (MenuBackground.LoadingScreen1..5), its top-left 1024 x 768 texels
//!   (SubXL / SubYL) stretched over the whole screen;
//! - "Deploying to <map>" (loadingMapPrefix, then the map name run
//!   through StripMap, StripPrefix and AddSpaces), white with a black
//!   shadow 1 pixel right and down (RODrawOpShadowedText), font
//!   fntROMainMenu (ROFonts.ROMain18), left-aligned, top at 0.91 of the
//!   height, 0.05 in from the left;
//! - nothing else: SetText empties ". . . LOADING" and the hint, and the
//!   VAC lines only appear on a VAC-secured server (never ours).
//!
//! **Handshake** (the map change wires it; this module only draws):
//! 1. send [`ShowLoadingScreen`]: the screen appears the same frame;
//! 2. after it has been on screen for [`SHOWN_AFTER_FRAMES`] frames the
//!    module sends [`LoadingScreenShown`]: start the (blocking) load on it,
//!    the picture then stays on the display while the game is frozen;
//! 3. send [`HideLoadingScreen`] when the new map is ready.
//!
//! `--loading-test MAP` shows it over the running map (see [`LoadingTest`]).

use std::collections::HashMap;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ue_assets::package_set::PackageSet;

use crate::engine::runlog;
use crate::game::hud::{Canvas, HudFont, HudTexture, Loader};
use crate::world::map::MapRequest;

/// ROServerLoading.loadingMapPrefix (its default; no System/*.int file
/// changes it).
pub const LOADING_MAP_PREFIX: &str = "Deploying to";

/// ROServerLoading.Backgrounds (its own defaults: User.ini only has a
/// section for the parent class, GUI2K4.UT2K4ServerLoading).
pub const BACKGROUNDS: [&str; 5] = [
    "MenuBackground.LoadingScreen1",
    "MenuBackground.LoadingScreen2",
    "MenuBackground.LoadingScreen3",
    "MenuBackground.LoadingScreen4",
    "MenuBackground.LoadingScreen5",
];

/// fntROMainMenu.FontArrayNames: the same font for both entries.
const FONT: &str = "ROFonts.ROMain18";
/// fntROMainMenu: bScaled, NormalXRes 800, FallBackRes 512.
const FONT_NORMAL_X_RES: f32 = 800.0;
const FONT_FALLBACK_RES: f32 = 512.0;
/// DrawOpText.Draw / RODrawOpShadowedText.Draw: Canvas.FontScaleX = 0.9.
const CANVAS_FONT_SCALE: f32 = 0.9;
/// UT2K4ServerLoading.OpBackground: SubXL 1024, SubYL 768.
const BACKGROUND_TEXELS: Vec2 = Vec2::new(1024.0, 768.0);
/// ROServerLoading.OpMapname: Top 0.91, Lft 0.05.
const TEXT_TOP: f32 = 0.91;
const TEXT_LEFT: f32 = 0.05;
/// RODrawOpShadowedText: ShadowColor black, shadowXOffset / YOffset 1.
const SHADOW_OFFSET: Vec2 = Vec2::new(1.0, 1.0);
/// UT2K4ServerLoading.SetImage: at most 10 picks before giving up.
const MAX_PICKS: u32 = 10;

/// Frames the screen is drawn before [`LoadingScreenShown`] is sent. Bevy
/// draws a frame while the next one is being updated (pipelined
/// rendering), so when the message is read (the frame after the third)
/// the first two frames with the screen have been drawn and presented,
/// and the one being drawn also shows it.
pub const SHOWN_AFTER_FRAMES: u32 = 3;

/// Above everything else on screen (the menus use 1000 + up to 3000).
const Z_BACKGROUND: i32 = 10_000;
/// Glyph nodes: shadow and white text, one per character.
const GLYPH_SLOTS: usize = 256;

/// Shows the loading screen for `map` (e.g. "KF-WestLondon"). A second
/// one while it is up starts over (new picture, new text, the frame count
/// from zero; [`LoadingScreenShown`] is sent again).
#[derive(Message, Clone, Debug)]
pub struct ShowLoadingScreen {
    pub map: String,
}

/// Sent once the screen for `map` has been on screen for
/// [`SHOWN_AFTER_FRAMES`] frames: the map load may start now.
#[derive(Message, Clone, Debug)]
pub struct LoadingScreenShown {
    pub map: String,
}

/// Hides the loading screen (send it when the new map is ready).
#[derive(Message, Clone, Copy, Debug)]
pub struct HideLoadingScreen;

/// `--loading-test MAP`: shows the loading screen for MAP over the running
/// map at frame [`LoadingTest::SHOW_AT`], freezes the game for
/// [`LoadingTest::FREEZE`] when [`LoadingScreenShown`] arrives (a stand-in
/// for the map load), then hides it [`LoadingTest::HIDE_AFTER`] frames
/// later.
#[derive(Resource, Clone, Debug)]
pub struct LoadingTest {
    pub map: String,
}

impl LoadingTest {
    pub const SHOW_AT: u32 = 20;
    pub const FREEZE: std::time::Duration = std::time::Duration::from_millis(1500);
    pub const HIDE_AFTER: u32 = 60;
}

/// UT2K4ServerLoading.StripMap: drops the extension (after the last '.')
/// and any folder (up to the last '\', '/' or ':'). As in the script, the
/// first character is never looked at.
pub fn strip_map(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut s: Vec<char> = match (1..chars.len()).rev().find(|&p| chars[p] == '.') {
        Some(p) => chars[..p].to_vec(),
        None => chars,
    };
    if let Some(p) = (1..s.len()).rev().find(|&p| matches!(s[p], '\\' | '/' | ':')) {
        s = s[p + 1..].to_vec();
    }
    s.into_iter().collect()
}

/// ROServerLoading.StripPrefix: drops a leading "RO-" (Red Orchestra's
/// prefix; KF's "KF-" is left alone).
pub fn strip_prefix(s: &str) -> String {
    match s.strip_prefix("RO-") {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ => s.to_string(),
    }
}

/// ROServerLoading.AddSpaces: '_' becomes ' ', then a space goes before
/// every capital letter (a character whose Caps is itself and whose Locs
/// is not), except at the start. Digits, '-' and spaces are not capitals,
/// so "KF-WestLondon" becomes "K F- West London" as in KF.
pub fn add_spaces(s: &str) -> String {
    let temp: Vec<char> = s.replace('_', " ").chars().collect();
    if temp.len() <= 1 {
        return temp.into_iter().collect();
    }
    let is_capital = |c: char| c.to_uppercase().eq(std::iter::once(c)) && !c.to_lowercase().eq(std::iter::once(c));
    let mut result = String::new();
    let mut lastpos = 0;
    for (pos, &c) in temp.iter().enumerate() {
        if is_capital(c) {
            if !result.is_empty() {
                result.push(' ');
            }
            result.extend(&temp[lastpos..pos]);
            lastpos = pos;
        }
    }
    if lastpos != temp.len() {
        if !result.is_empty() {
            result.push(' ');
        }
        result.extend(&temp[lastpos..]);
    }
    result
}

/// ROServerLoading.SetText's map name: StripMap, StripPrefix, AddSpaces,
/// and its one exception ("HEDGE HOG" -> "Hedgehog").
pub fn map_title(map: &str) -> String {
    let m = add_spaces(&strip_prefix(&strip_map(map)));
    if m.to_uppercase() == "HEDGE HOG" { "Hedgehog".to_string() } else { m }
}

/// The text KF shows: `loadingMapPrefix @ Map` ('@' joins with a space).
pub fn loading_text(map: &str) -> String {
    format!("{LOADING_MAP_PREFIX} {}", map_title(map))
}

/// UT2K4ServerLoading.SetImage: Rand(Backgrounds.Length) until a picture
/// loads, at most [`MAX_PICKS`] times. `rand(n)` is Rand(n) (0 to n - 1),
/// `load` tries one path. Returns the index that loaded, or None (KF then
/// draws nothing behind the text; ours: black).
pub fn pick_background(list: &[&str], mut rand: impl FnMut(usize) -> usize, mut load: impl FnMut(&str) -> bool) -> Option<usize> {
    if list.is_empty() {
        return None;
    }
    for _ in 0..MAX_PICKS {
        let i = rand(list.len()) % list.len();
        // An empty entry keeps MenuBlack (a black picture) in KF.
        if list[i].is_empty() {
            return None;
        }
        if load(list[i]) {
            return Some(i);
        }
    }
    None
}

/// Where KF draws the text and how big, for a screen of `screen` physical
/// pixels: the top-left of the text and the glyph scale. GUIFont.GetFont
/// (native) for a scaled font: at widths up to FallBackRes the fallback
/// entry at scale 1, else the first entry at width / NormalXRes; the
/// canvas then multiplies by its FontScaleX (0.9). Details in the local
/// RE.md.
pub fn text_layout(screen: Vec2) -> (Vec2, f32) {
    let font_scale = if screen.x <= FONT_FALLBACK_RES { 1.0 } else { screen.x / FONT_NORMAL_X_RES };
    (Vec2::new(TEXT_LEFT * screen.x, TEXT_TOP * screen.y), font_scale * CANVAS_FONT_SCALE)
}

/// Rand() for the picture (xorshift); seeded from the clock at startup,
/// so the picture changes from run to run as in KF.
struct Rng(u32);

impl Rng {
    fn rand(&mut self, n: usize) -> usize {
        let r = &mut self.0;
        *r ^= *r << 13;
        *r ^= *r >> 17;
        *r ^= *r << 5;
        (*r as usize) % n.max(1)
    }
}

struct Showing {
    map: String,
    text: String,
    /// Index into `LoadingScreen::textures`, None: black.
    background: Option<usize>,
    frames_shown: u32,
    shown_sent: bool,
}

/// The loading screen's state and the pictures and font it has loaded
/// (loaded the first time they are needed, then kept).
#[derive(Resource)]
pub struct LoadingScreen {
    showing: Option<Showing>,
    rng: Rng,
    textures: Vec<HudTexture>,
    by_path: HashMap<String, Option<usize>>,
    font: Option<HudFont>,
    font_tried: bool,
}

impl Default for LoadingScreen {
    fn default() -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32) | 1;
        LoadingScreen { showing: None, rng: Rng(seed), textures: Vec::new(), by_path: HashMap::new(), font: None, font_tried: false }
    }
}

#[derive(Component)]
struct LoadingBackground;

#[derive(Component)]
struct LoadingGlyph(usize);

pub struct LoadingScreenPlugin;

impl Plugin for LoadingScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadingScreen>()
            .add_message::<ShowLoadingScreen>()
            .add_message::<LoadingScreenShown>()
            .add_message::<HideLoadingScreen>()
            .add_systems(Startup, spawn_nodes)
            .add_systems(Update, run_test.run_if(resource_exists::<LoadingTest>))
            .add_systems(PostUpdate, (receive, draw).chain());
    }
}

fn spawn_nodes(mut commands: Commands) {
    commands.spawn((
        Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        ImageNode { image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() },
        BackgroundColor(Color::BLACK),
        Visibility::Hidden,
        GlobalZIndex(Z_BACKGROUND),
        LoadingBackground,
    ));
    for i in 0..GLYPH_SLOTS {
        commands.spawn((
            Node { position_type: PositionType::Absolute, ..default() },
            ImageNode { image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() },
            Visibility::Hidden,
            GlobalZIndex(Z_BACKGROUND + 1 + i as i32),
            LoadingGlyph(i),
        ));
    }
}

/// Loads one picture or font page set into the screen's own texture list.
fn with_loader<T>(screen: &mut LoadingScreen, set: &PackageSet, images: &mut Assets<Image>, f: impl FnOnce(&mut Loader) -> T) -> T {
    let mut loader = Loader { set, images, textures: std::mem::take(&mut screen.textures), by_path: std::mem::take(&mut screen.by_path), missing: Vec::new() };
    let out = f(&mut loader);
    screen.textures = loader.textures;
    screen.by_path = loader.by_path;
    out
}

fn receive(
    mut show: MessageReader<ShowLoadingScreen>,
    mut hide: MessageReader<HideLoadingScreen>,
    mut screen: ResMut<LoadingScreen>,
    request: Res<MapRequest>,
    mut images: ResMut<Assets<Image>>,
    frames: Res<FrameCount>,
) {
    let shows: Vec<ShowLoadingScreen> = show.read().cloned().collect();
    let hides = hide.read().count();
    // A hide sent with (after) a show in the same frame: the show is dropped.
    if hides > 0 {
        match screen.showing.take() {
            Some(s) => runlog::kv("loading_screen_hide", &format!("map={} frames_shown={} frame={}", s.map, s.frames_shown, frames.0)),
            None => runlog::kv("loading_screen_hide", &format!("ignored=not_shown frame={}", frames.0)),
        }
        return;
    }
    let Some(ShowLoadingScreen { map }) = shows.into_iter().last() else { return };
    let started = std::time::Instant::now();
    let set = PackageSet::new(&request.install_root);
    let screen = &mut *screen;
    if !screen.font_tried {
        screen.font_tried = true;
        screen.font = load_font(screen, &set, &mut images);
    }
    // Rand() and the loads share the screen: the generator is taken out
    // while the loader holds the texture list.
    let mut rng = std::mem::replace(&mut screen.rng, Rng(1));
    let mut tried = 0;
    let mut loaded = None;
    let pick = with_loader(screen, &set, &mut images, |loader| {
        pick_background(&BACKGROUNDS, |n| rng.rand(n), |path| {
            tried += 1;
            loaded = loader.texture_path(path);
            loaded.is_some()
        })
    });
    screen.rng = rng;
    let background = pick.and(loaded);
    let text = loading_text(&map);
    let size = background.map_or(Vec2::ZERO, |t| screen.textures[t].size);
    runlog::kv(
        "loading_screen_show",
        &format!(
            "map={map} background={} texels={}x{} picks={} text=\"{text}\" font={} frame={} load_ms={:.1}",
            pick.map_or("none(black)", |i| BACKGROUNDS[i]),
            size.x,
            size.y,
            tried,
            if screen.font.is_some() { FONT } else { "missing" },
            frames.0,
            started.elapsed().as_secs_f64() * 1000.0
        ),
    );
    screen.showing = Some(Showing { map, text, background, frames_shown: 0, shown_sent: false });
}

fn load_font(screen: &mut LoadingScreen, set: &PackageSet, images: &mut Assets<Image>) -> Option<HudFont> {
    let Some(h) = set.find_object(FONT, Some("Font")) else {
        runlog::kv("loading_screen_font", &format!("font={FONT} missing=not_found"));
        return None;
    };
    let font = match ue_assets::font::read_font(&h.package.pkg, h.export) {
        Ok(f) => f,
        Err(e) => {
            runlog::kv("loading_screen_font", &format!("font={FONT} missing=\"{e}\""));
            return None;
        }
    };
    let pages = with_loader(screen, set, images, |l| font.textures.iter().map(|&t| l.texture(&h.package, t)).collect());
    Some(HudFont { name: FONT.to_string(), font, pages })
}

type BackgroundNode = (&'static mut ImageNode, &'static mut Visibility);

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn draw(
    mut screen: ResMut<LoadingScreen>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut background: Query<BackgroundNode, (With<LoadingBackground>, Without<LoadingGlyph>)>,
    mut glyphs: Query<(&LoadingGlyph, &mut Node, &mut ImageNode, &mut Visibility), Without<LoadingBackground>>,
    mut shown: MessageWriter<LoadingScreenShown>,
    frames: Res<FrameCount>,
    mut glyphs_up: Local<usize>,
) {
    let screen = &mut *screen;
    let Ok((mut bg_image, mut bg_vis)) = background.single_mut() else { return };
    let (Some(s), Ok(win)) = (screen.showing.as_mut(), window.single()) else {
        if *bg_vis != Visibility::Hidden {
            *bg_vis = Visibility::Hidden;
        }
        if *glyphs_up > 0 {
            for (_, _, _, mut vis) in glyphs.iter_mut() {
                *vis = Visibility::Hidden;
            }
            *glyphs_up = 0;
        }
        return;
    };
    // Background: the texture's top-left 1024 x 768 over the whole screen.
    match s.background {
        Some(t) => {
            let tex = &screen.textures[t];
            if bg_image.image != tex.image {
                bg_image.image = tex.image.clone();
            }
            bg_image.rect = Some(Rect::from_corners(Vec2::ZERO, BACKGROUND_TEXELS.min(tex.size)));
            bg_image.color = Color::WHITE;
        }
        None => {
            bg_image.image = Handle::default();
            bg_image.color = Color::NONE;
        }
    }
    *bg_vis = Visibility::Inherited;

    // Text: the shadow pass, then the white pass (RODrawOpShadowedText).
    let physical = Vec2::new(win.physical_width() as f32, win.physical_height() as f32);
    let mut canvas = Canvas::new(Vec2::new(win.width(), win.height()), 255);
    canvas.scale_factor = win.scale_factor();
    let (at, scale) = text_layout(physical);
    if let Some(font) = screen.font.as_ref() {
        canvas.text(font, &s.text, at + SHADOW_OFFSET, scale, [0, 0, 0, 255], "loading_shadow");
        canvas.text(font, &s.text, at, scale, [255, 255, 255, 255], "loading_text");
    }
    let quads = &canvas.quads;
    let n = quads.len().min(GLYPH_SLOTS);
    for (slot, mut node, mut image, mut vis) in glyphs.iter_mut() {
        let Some(q) = quads.get(slot.0).filter(|_| slot.0 < n) else {
            *vis = Visibility::Hidden;
            continue;
        };
        node.left = Val::Px(q.screen.min.x);
        node.top = Val::Px(q.screen.min.y);
        node.width = Val::Px(q.screen.width());
        node.height = Val::Px(q.screen.height());
        let t = &screen.textures[q.texture];
        if image.image != t.image {
            image.image = t.image.clone();
        }
        image.rect = Some(q.uv);
        image.color = Color::srgba_u8(q.tint[0], q.tint[1], q.tint[2], q.tint[3]);
        *vis = Visibility::Inherited;
    }
    *glyphs_up = n;

    s.frames_shown += 1;
    if s.frames_shown == 1 {
        runlog::kv(
            "loading_screen_layout",
            &format!(
                "screen={}x{} text_at=({:.0},{:.0}) font_scale={scale:.3} glyph_quads={} text_width={:.0}",
                physical.x,
                physical.y,
                at.x,
                at.y,
                quads.len(),
                screen.font.as_ref().map_or(0.0, |f| Canvas::text_size(f, &s.text, scale).x)
            ),
        );
    }
    if !s.shown_sent && s.frames_shown >= SHOWN_AFTER_FRAMES {
        s.shown_sent = true;
        shown.write(LoadingScreenShown { map: s.map.clone() });
        runlog::kv("loading_screen_shown", &format!("map={} frames_shown={} frame={}", s.map, s.frames_shown, frames.0));
    }
}

/// `--loading-test`: show at SHOW_AT, freeze on LoadingScreenShown, hide
/// HIDE_AFTER frames later.
fn run_test(
    test: Res<LoadingTest>,
    frames: Res<FrameCount>,
    mut show: MessageWriter<ShowLoadingScreen>,
    mut shown: MessageReader<LoadingScreenShown>,
    mut hide: MessageWriter<HideLoadingScreen>,
    mut hide_at: Local<Option<u32>>,
) {
    if frames.0 == LoadingTest::SHOW_AT {
        show.write(ShowLoadingScreen { map: test.map.clone() });
        runlog::kv("loading_test", &format!("step=show map={} frame={}", test.map, frames.0));
    }
    for m in shown.read() {
        let started = std::time::Instant::now();
        std::thread::sleep(LoadingTest::FREEZE);
        runlog::kv("loading_test", &format!("step=freeze map={} frame={} frozen_ms={:.0}", m.map, frames.0, started.elapsed().as_secs_f64() * 1000.0));
        *hide_at = Some(frames.0 + LoadingTest::HIDE_AFTER);
    }
    if *hide_at == Some(frames.0) {
        *hide_at = None;
        hide.write(HideLoadingScreen);
        runlog::kv("loading_test", &format!("step=hide frame={}", frames.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_names_follow_rose_set_text() {
        // KF keeps "KF-": StripPrefix only drops "RO-".
        assert_eq!(map_title("KF-WestLondon"), "K F- West London");
        assert_eq!(map_title("KF-BioticsLab"), "K F- Biotics Lab");
        assert_eq!(map_title("KF-Farm.rom"), "K F- Farm");
        assert_eq!(loading_text("KF-Manor"), "Deploying to K F- Manor");
        assert_eq!(map_title("RO-Arad"), "Arad");
        assert_eq!(map_title("RO-HedgeHog"), "Hedgehog");
        // Digits and lower case are not capitals; '_' becomes a space.
        assert_eq!(map_title("KF-Map2Test"), "K F- Map2 Test");
        assert_eq!(map_title("KF-Foo_bar"), "K F- Foo bar");
        assert_eq!(map_title("Hospital_Horrors"), "Hospital  Horrors");
        assert_eq!(map_title("lowerCase"), "lower Case");
        assert_eq!(map_title("X"), "X");
        assert_eq!(map_title(""), "");
    }

    #[test]
    fn strip_map_drops_folder_and_extension() {
        assert_eq!(strip_map("KF-Farm.rom"), "KF-Farm");
        assert_eq!(strip_map("Maps/KF-Farm.rom"), "KF-Farm");
        assert_eq!(strip_map("C:\\KF\\Maps\\KF-Offices.rom"), "KF-Offices");
        assert_eq!(strip_map("KF-Farm"), "KF-Farm");
        // The first character is never looked at.
        assert_eq!(strip_map(".rom"), ".rom");
    }

    #[test]
    fn strip_prefix_needs_something_after() {
        assert_eq!(strip_prefix("RO-"), "RO-");
        assert_eq!(strip_prefix("RO-Odessa"), "Odessa");
        assert_eq!(strip_prefix("KF-Manor"), "KF-Manor");
    }

    #[test]
    fn background_pick_retries_up_to_ten_times() {
        // Seeded draws: picks 3 first; it loads.
        let mut draws = [3usize, 1].into_iter();
        assert_eq!(pick_background(&BACKGROUNDS, |_| draws.next().unwrap(), |_| true), Some(3));
        // Nothing loads: ten tries, then none.
        let mut tries = 0;
        assert_eq!(
            pick_background(&BACKGROUNDS, |n| n - 1, |_| {
                tries += 1;
                false
            }),
            None
        );
        assert_eq!(tries, 10);
        // The second pick loads.
        let mut seen = Vec::new();
        let mut draws = [0usize, 4].into_iter();
        let got = pick_background(&BACKGROUNDS, |_| draws.next().unwrap(), |p| {
            seen.push(p.to_string());
            p.ends_with('5')
        });
        assert_eq!(got, Some(4));
        assert_eq!(seen, ["MenuBackground.LoadingScreen1", "MenuBackground.LoadingScreen5"]);
        // The seeded generator spreads over the list.
        let mut rng = Rng(0x1234_5678);
        let mut hit = [false; 5];
        for _ in 0..200 {
            hit[rng.rand(5)] = true;
        }
        assert!(hit.iter().all(|&h| h));
    }

    #[test]
    fn text_sits_at_kf_fractions_and_scales_with_width() {
        let (at, scale) = text_layout(Vec2::new(1920.0, 1080.0));
        assert!((at - Vec2::new(96.0, 982.8)).length() < 1e-3);
        assert!((scale - 0.9 * 1920.0 / 800.0).abs() < 1e-5);
        // At 512 wide and below: the fallback entry at its own size.
        assert_eq!(text_layout(Vec2::new(512.0, 384.0)).1, 0.9);
        assert!((text_layout(Vec2::new(800.0, 600.0)).1 - 0.9).abs() < 1e-6);
    }
}
