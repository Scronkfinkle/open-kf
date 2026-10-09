//! The loading screen shown while a map loads (map rotation plan, step 5).
//!
//! KF has two loading screens (GameEngine's ConnectingMenuClass and
//! LoadingClass). The engine draws LoadingClass (ROServerLoading,
//! "Deploying to <map>") only in a single-player ladder game with
//! TeamScreen=true, which KF never plays; every other map load, map
//! changes included, draws ConnectingMenuClass,
//! GUI2K4.UT2K4ServerLoading, with KF's hint and map picture added by
//! KFGameType.GetLoadingHint. That one is drawn here (details in the local
//! RE.md). Positions are fractions of the screen:
//! - a background picked at random from UT2K4ServerLoading.Backgrounds
//!   (defuser.ini: 2k4Menus.Loading.loadingscreen1, 2, 2, 4), its top-left
//!   1024 x 768 texels (OpBackground SubXL / SubYL) over the whole screen;
//! - ". . . LOADING" (OpLoading, GUI2K4.int) right-aligned in the box
//!   from 0.5 to 0.99 across, top at 0.48, font UT2LargeFont;
//! - the map name as the engine gives it (StripMap: no folder, no
//!   extension, e.g. "KF-WestLondon") the same way at 0.6 (OpMapname);
//! - a random KFGameType.KFHints line (OpHint, fntUT2k4SmallHeader),
//!   wrapped to 0.93 of the width from 0.05, each line right-aligned,
//!   top at 0.8;
//! - the map's preview (its LevelSummary ScreenShot, a random picture of
//!   a MaterialSequence), 3/7 of the screen wide and tall, right of the
//!   centre near the top, with the map title and "By <author>" on it
//!   (KFMod.LoadingInfoImage).
//!
//! All text is white (DrawOpBase.DrawColor) at the canvas font scale 0.9.
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
use ue_assets::package::ObjectRef;
use ue_assets::package_set::PackageSet;
use ue_assets::properties::Value;

use crate::engine::runlog;
use crate::game::hud::{Canvas, HudFont, HudTexture, Loader, Quad};
use crate::game::menus::gui;
use crate::world::map::MapRequest;

/// UT2K4ServerLoading.Backgrounds when defuser.ini cannot be read (the
/// same list; KF's fresh-install User.ini is made from defuser.ini).
pub const DEFAULT_BACKGROUNDS: [&str; 4] =
    ["2k4Menus.Loading.loadingscreen1", "2k4Menus.Loading.loadingscreen2", "2k4Menus.Loading.loadingscreen2", "2k4Menus.Loading.loadingscreen4"];

/// GUI2K4.int [UT2K4ServerLoading] OpLoading.Text.
const LOADING_TEXT: &str = ". . . LOADING";
/// fntUT2k4Large (UT2LargeFont) and fntUT2k4SmallHeader FontArrayNames
/// (GUI2K4.int).
const LARGE_FONTS: [&str; 5] = ["ROFonts.ROBtsrmVr14", "ROFonts.ROBtsrmVr16", "ROFonts.ROBtsrmVr18", "ROFonts.ROBtsrmVr20", "ROFonts.ROBtsrmVr22"];
const SMALL_HEADER_FONTS: [&str; 5] = ["ROFontsTwo.ROArial12DS", "ROFontsTwo.ROArial14DS", "ROFontsTwo.ROArial18DS", "ROFontsTwo.ROArial18DS", "ROFontsTwo.ROArial22DS"];
/// LoadingInfoImage: HUDKillingFloor.LoadFontStatic(3) above 580 pixels
/// tall, else (2): HUD.FontArrayNames[3] / [2].
const TITLE_FONT_TALL: &str = "ROFontsTwo.ROArial18DS";
const TITLE_FONT_SHORT: &str = "ROFontsTwo.ROArial22DS";
/// DrawOpText.Draw: Canvas.FontScaleX / Y = 0.9 (and the fonts here are
/// not scaled GUIFonts, so their own scale is 1).
const FONT_SCALE: f32 = 0.9;
/// UT2K4ServerLoading.OpBackground: SubXL 1024, SubYL 768.
const BACKGROUND_TEXELS: Vec2 = Vec2::new(1024.0, 768.0);
/// LevelInfo's default Title and Author: LoadingInfoImage draws neither
/// when the map kept them.
const DEFAULT_TITLE: &str = "Untitled";
const DEFAULT_AUTHOR: &str = "Anonymous";
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
/// Nodes over the background: the preview and one per character.
const SLOTS: usize = 1024;

/// Shows the loading screen for `map` (e.g. "KF-WestLondon"). A second
/// one while it is up starts over (new picture and hint, the frame count
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

/// Hides the loading screen (send it when the new map is ready). A hide
/// in the same frame as a show wins.
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

/// UT2K4ServerLoading.SetImage: Rand(Backgrounds.Length) until a picture
/// loads, at most [`MAX_PICKS`] times. `rand(n)` is Rand(n) (0 to n - 1),
/// `load` tries one path. Returns the index that loaded, or None (KF then
/// draws no background; ours: black).
pub fn pick_background(list: &[String], mut rand: impl FnMut(usize) -> usize, mut load: impl FnMut(&str) -> bool) -> Option<usize> {
    if list.is_empty() {
        return None;
    }
    for _ in 0..MAX_PICKS {
        let i = rand(list.len()) % list.len();
        // An empty entry keeps MenuBlack (a black picture) in KF.
        if list[i].is_empty() {
            return None;
        }
        if load(&list[i]) {
            return Some(i);
        }
    }
    None
}

/// The quoted strings of an .int / .ini array value: `("a","b")`.
pub fn quoted_list(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => s.extend(chars.next()),
                '"' => break,
                c => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

/// GUIFont.GetFont for a font that is not scaled (native): which of the
/// five sizes a canvas this wide uses. Details in the local RE.md.
pub fn font_size_index(width: f32) -> usize {
    if width < 800.0 {
        0
    } else if width < 1024.0 {
        1
    } else if width < 1280.0 {
        2
    } else if width < 1600.0 {
        3
    } else {
        4
    }
}

/// Where KF draws each part on a screen of `screen` physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// OpLoading and OpMapname: right edge (0.99 of the width) and tops.
    pub right: f32,
    pub loading_top: f32,
    pub map_top: f32,
    /// OpHint: left, width (lines wrap to it and are right-aligned in it), top.
    pub hint_left: f32,
    pub hint_width: f32,
    pub hint_top: f32,
    /// LoadingInfoImage: the preview box, the title's and author's spots.
    pub preview: Rect,
    pub title_at: Vec2,
    pub author_at: Vec2,
    pub large_font: &'static str,
    pub hint_font: &'static str,
    pub title_font: &'static str,
}

/// The positions of UT2K4ServerLoading's draw ops (Lft + Width, Top) and
/// LoadingInfoImage.Draw (integer X, Y; XS = ClipX / 7 * 3).
pub fn layout(screen: Vec2) -> Layout {
    let (w, h) = (screen.x, screen.y);
    let xs = w / 7.0 * 3.0;
    let ys = h / 7.0 * 3.0;
    let half = (w / 2.0).trunc();
    let x = (half + (half - xs) / 2.0).trunc();
    let y = ((h / 2.0 - ys) / 5.0 * 3.0).trunc();
    let tall = h > 580.0;
    let size = font_size_index(w);
    Layout {
        right: (0.5 + 0.49) * w,
        loading_top: 0.48 * h,
        map_top: 0.6 * h,
        hint_left: 0.05 * w,
        hint_width: 0.93 * w,
        hint_top: 0.8 * h,
        preview: Rect::new(x, y, x + xs, y + ys),
        title_at: Vec2::new(x + 4.0, y + 3.0),
        author_at: Vec2::new(x + 14.0, y + 3.0 + if tall { 22.0 } else { 18.0 }),
        large_font: LARGE_FONTS[size],
        hint_font: SMALL_HEADER_FONTS[size],
        title_font: if tall { TITLE_FONT_TALL } else { TITLE_FONT_SHORT },
    }
}

/// Canvas.WrapStringToArray (native; assumed): words onto lines no wider
/// than `width`, '|' starts a new line, a word wider than a line stays
/// whole. `measure` gives a string's width.
pub fn wrap(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('|') {
        let mut line = String::new();
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && measure(&candidate) > width {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        out.push(line);
    }
    out
}

/// Rand() for the picks (xorshift); seeded from the clock at startup, so
/// the picture and hint change from run to run as in KF.
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

/// What one showing draws (fixed when it is shown).
struct Showing {
    map: String,
    map_name: String,
    hint: String,
    /// Indices into `LoadingScreen::textures`; None: black / none.
    background: Option<usize>,
    preview: Option<usize>,
    title: String,
    author: String,
    frames_shown: u32,
    shown_sent: bool,
}

/// The loading screen's state and what it has loaded (pictures and fonts
/// are loaded the first time they are needed, then kept).
#[derive(Resource)]
pub struct LoadingScreen {
    showing: Option<Showing>,
    rng: Rng,
    textures: Vec<HudTexture>,
    by_path: HashMap<String, Option<usize>>,
    fonts: HashMap<&'static str, Option<HudFont>>,
    /// Backgrounds and KFHints, read once from the install.
    backgrounds: Vec<String>,
    hints: Vec<String>,
    texts_read: bool,
}

impl Default for LoadingScreen {
    fn default() -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32) | 1;
        LoadingScreen {
            showing: None,
            rng: Rng(seed),
            textures: Vec::new(),
            by_path: HashMap::new(),
            fonts: HashMap::new(),
            backgrounds: Vec::new(),
            hints: Vec::new(),
            texts_read: false,
        }
    }
}

#[derive(Component)]
struct LoadingBackground;

#[derive(Component)]
struct LoadingSlot(usize);

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
    for i in 0..SLOTS {
        commands.spawn((
            Node { position_type: PositionType::Absolute, ..default() },
            ImageNode { image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() },
            Visibility::Hidden,
            GlobalZIndex(Z_BACKGROUND + 1 + i as i32),
            LoadingSlot(i),
        ));
    }
}

/// Runs `f` with a texture loader over the screen's own texture list.
fn with_loader<T>(screen: &mut LoadingScreen, set: &PackageSet, images: &mut Assets<Image>, f: impl FnOnce(&mut Loader) -> T) -> T {
    let mut loader = Loader { set, images, textures: std::mem::take(&mut screen.textures), by_path: std::mem::take(&mut screen.by_path), missing: Vec::new() };
    let out = f(&mut loader);
    screen.textures = loader.textures;
    screen.by_path = loader.by_path;
    out
}

/// Reads Backgrounds (defuser.ini) and KFHints (KFMod.int) once.
fn read_texts(screen: &mut LoadingScreen, root: &std::path::Path) {
    if screen.texts_read {
        return;
    }
    screen.texts_read = true;
    let defuser = gui::read_latin1(&root.join("System").join("defuser.ini"));
    screen.backgrounds = crate::audio::music::int_section(&defuser, "GUI2K4.UT2K4ServerLoading")
        .into_iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("Backgrounds"))
        .map(|(_, v)| v.trim().to_string())
        .collect();
    if screen.backgrounds.is_empty() {
        screen.backgrounds = DEFAULT_BACKGROUNDS.iter().map(|s| s.to_string()).collect();
    }
    let kfmod = gui::read_latin1(&root.join("System").join("KFMod.int"));
    screen.hints = crate::audio::music::int_section(&kfmod, "KFGameType")
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("KFHints"))
        .map(|(_, v)| quoted_list(&v))
        .unwrap_or_default();
    runlog::kv("loading_screen_texts", &format!("backgrounds=[{}] hints={} rng_seed={}", screen.backgrounds.join(" "), screen.hints.len(), screen.rng.0));
}

/// Loads the fonts a layout uses that are not loaded yet (the first
/// frame, and after the window changes size class).
fn ensure_fonts(screen: &mut LoadingScreen, root: &std::path::Path, images: &mut Assets<Image>, lay: &Layout) {
    let names = [lay.large_font, lay.hint_font, lay.title_font];
    if names.iter().all(|n| screen.fonts.contains_key(n)) {
        return;
    }
    let set = PackageSet::new(root);
    for name in names {
        if screen.fonts.contains_key(name) {
            continue;
        }
        let loaded = set.find_object(name, Some("Font")).and_then(|h| {
            let font = ue_assets::font::read_font(&h.package.pkg, h.export).ok()?;
            let pages = with_loader(screen, &set, images, |l| font.textures.iter().map(|&t| l.texture(&h.package, t)).collect());
            Some(HudFont { name: name.to_string(), font, pages })
        });
        if loaded.is_none() {
            runlog::kv("loading_screen_font", &format!("font={name} missing=true"));
        }
        screen.fonts.insert(name, loaded);
    }
}

/// KFGameType.GetLoadingHint's picture: the map's LevelSummary
/// ScreenShot (a MaterialSequence: one of its items at random), its Title
/// (System/<map>.int [LevelSummary] first, as a localized value) and
/// Author. None when the map has no picture.
fn read_preview(screen: &mut LoadingScreen, set: &PackageSet, images: &mut Assets<Image>, root: &std::path::Path, map: &str) -> (Option<usize>, String, String, String) {
    let Some(ls) = set.find_object(&format!("{map}.LevelSummary"), Some("LevelSummary")) else {
        return (None, String::new(), String::new(), "no_level_summary".into());
    };
    let pkg = &ls.package;
    let Ok(props) = ue_assets::properties::read_export_properties(&pkg.pkg, ls.export) else {
        return (None, String::new(), String::new(), "unreadable_level_summary".into());
    };
    let text = |name: &str| match props.get(&pkg.pkg, name) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let map_int = gui::read_latin1(&root.join("System").join(format!("{map}.int")));
    let title = gui::ini_value(&map_int, "LevelSummary", "Title").unwrap_or_else(|| text("Title"));
    let author = text("Author");
    let Some(Value::Object(shot)) = props.get(&pkg.pkg, "ScreenShot").cloned() else {
        return (None, title, author, "no_screenshot".into());
    };
    let Some(h) = set.resolve(pkg, shot) else {
        return (None, title, author, "screenshot_not_found".into());
    };
    // A MaterialSequence: TexToUse[Rand(j)] over its SequenceItems.
    let mut chosen = (h.package.clone(), ObjectRef::Export(h.export), h.path());
    if h.class_name().eq_ignore_ascii_case("MaterialSequence")
        && let Ok(seq) = ue_assets::properties::read_export_properties_ext(&h.package.pkg, h.export, &["SequenceItems"])
        && let Some(Value::StructArray(items)) = seq.get(&h.package.pkg, "SequenceItems")
    {
        let mats: Vec<ObjectRef> = items
            .iter()
            .filter_map(|it| match it.get(&h.package.pkg, "Material") {
                Some(Value::Object(rf)) if *rf != ObjectRef::Null => Some(*rf),
                _ => None,
            })
            .collect();
        if !mats.is_empty() {
            let i = screen.rng.rand(mats.len());
            let path = set.resolve(&h.package, mats[i]).map_or_else(|| format!("{:?}", mats[i]), |x| x.path());
            chosen = (h.package.clone(), mats[i], format!("{}[{i}_of_{}]={path}", h.path(), mats.len()));
        }
    }
    let (p, rf, what) = chosen;
    let tex = with_loader(screen, set, images, |l| l.texture(&p, rf));
    (tex, title, author, what)
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
    if hide.read().count() > 0 {
        match screen.showing.take() {
            Some(s) => runlog::kv("loading_screen_hide", &format!("map={} frames_shown={} frame={}", s.map, s.frames_shown, frames.0)),
            None => runlog::kv("loading_screen_hide", &format!("ignored=not_shown frame={}", frames.0)),
        }
        return;
    }
    let Some(ShowLoadingScreen { map }) = shows.into_iter().last() else { return };
    let started = std::time::Instant::now();
    let root = &request.install_root;
    let set = PackageSet::new(root);
    let screen = &mut *screen;
    read_texts(screen, root);
    // Rand() and the loads share the screen: the generator and the list
    // are taken out while the loader holds the texture list.
    let mut rng = std::mem::replace(&mut screen.rng, Rng(1));
    let list = std::mem::take(&mut screen.backgrounds);
    let mut tried = 0;
    let mut loaded = None;
    let pick = with_loader(screen, &set, &mut images, |loader| {
        pick_background(&list, |n| rng.rand(n), |path| {
            tried += 1;
            loaded = loader.texture_path(path);
            loaded.is_some()
        })
    });
    screen.backgrounds = list;
    let hint = if screen.hints.is_empty() { String::new() } else { screen.hints[rng.rand(screen.hints.len())].clone() };
    screen.rng = rng;
    let background = pick.and(loaded);
    let map_name = strip_map(&map);
    let (preview, title, author, preview_what) = read_preview(screen, &set, &mut images, root, &map_name);
    let bg_size = background.map_or(Vec2::ZERO, |t| screen.textures[t].size);
    let pv_size = preview.map_or(Vec2::ZERO, |t| screen.textures[t].size);
    runlog::kv(
        "loading_screen_show",
        &format!(
            "map={map} background={} texels={}x{} picks={tried} text=\"{LOADING_TEXT}\" map_name=\"{map_name}\" preview={preview_what} preview_texels={}x{} title=\"{title}\" author=\"{author}\" hint=\"{}\" frame={} load_ms={:.1}",
            pick.map_or("none(black)", |i| screen.backgrounds[i].as_str()),
            bg_size.x,
            bg_size.y,
            pv_size.x,
            pv_size.y,
            hint.chars().take(60).collect::<String>(),
            frames.0,
            started.elapsed().as_secs_f64() * 1000.0
        ),
    );
    screen.showing = Some(Showing { map, map_name, hint, background, preview, title, author, frames_shown: 0, shown_sent: false });
}

/// One frame of the screen's quads (over the background), in draw order.
fn build_quads(screen: &LoadingScreen, lay: &Layout, logical: Vec2, scale_factor: f32) -> Vec<Quad> {
    let mut canvas = Canvas::new(logical, 255);
    canvas.scale_factor = scale_factor;
    let white = [255, 255, 255, 255];
    let Some(s) = screen.showing.as_ref() else { return Vec::new() };
    let font = |name: &str| screen.fonts.get(name).and_then(|f| f.as_ref());
    if let Some(f) = font(lay.large_font) {
        for (text, top, what) in [(LOADING_TEXT, lay.loading_top, "loading"), (s.map_name.as_str(), lay.map_top, "map_name")] {
            let w = Canvas::text_size(f, text, FONT_SCALE).x;
            canvas.text(f, text, Vec2::new((lay.right - w).round(), top.round()), FONT_SCALE, white, what);
        }
    }
    if let Some(f) = font(lay.hint_font) {
        let line_h = Canvas::text_size(f, "Wqg|", FONT_SCALE).y;
        for (i, line) in wrap(&s.hint, lay.hint_width, |t| Canvas::text_size(f, t, FONT_SCALE).x).iter().enumerate() {
            let w = Canvas::text_size(f, line, FONT_SCALE).x;
            canvas.text(f, line, Vec2::new((lay.hint_left + lay.hint_width - w).round(), (lay.hint_top + i as f32 * line_h).round()), FONT_SCALE, white, "hint");
        }
    }
    if let Some(t) = s.preview {
        let size = screen.textures[t].size;
        canvas.quads.push(Quad {
            texture: t,
            uv: Rect::from_corners(Vec2::ZERO, size),
            screen: Rect::from_corners(lay.preview.min / scale_factor, lay.preview.max / scale_factor),
            tint: white,
            what: "preview".into(),
        });
        let show_title = !s.title.is_empty() && !s.title.eq_ignore_ascii_case(DEFAULT_TITLE);
        if show_title && let Some(f) = font(lay.title_font) {
            canvas.text(f, &s.title, lay.title_at, FONT_SCALE, white, "title");
            if !s.author.is_empty() && s.author != DEFAULT_AUTHOR {
                canvas.text(f, &format!("By {}", s.author), lay.author_at, FONT_SCALE, white, "author");
            }
        }
    }
    canvas.quads
}

type BackgroundNode = (&'static mut ImageNode, &'static mut Visibility);

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn draw(
    mut screen: ResMut<LoadingScreen>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut background: Query<BackgroundNode, (With<LoadingBackground>, Without<LoadingSlot>)>,
    mut slots: Query<(&LoadingSlot, &mut Node, &mut ImageNode, &mut Visibility), Without<LoadingBackground>>,
    mut shown: MessageWriter<LoadingScreenShown>,
    frames: Res<FrameCount>,
    request: Res<MapRequest>,
    mut images: ResMut<Assets<Image>>,
    mut slots_up: Local<usize>,
) {
    let Ok((mut bg_image, mut bg_vis)) = background.single_mut() else { return };
    let (true, Ok(win)) = (screen.showing.is_some(), window.single()) else {
        if *bg_vis != Visibility::Hidden {
            *bg_vis = Visibility::Hidden;
        }
        if *slots_up > 0 {
            for (_, _, _, mut vis) in slots.iter_mut() {
                *vis = Visibility::Hidden;
            }
            *slots_up = 0;
        }
        return;
    };
    let screen = &mut *screen;
    // Background: the texture's top-left 1024 x 768 over the whole screen.
    match screen.showing.as_ref().and_then(|s| s.background) {
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

    let physical = Vec2::new(win.physical_width() as f32, win.physical_height() as f32);
    let lay = layout(physical);
    ensure_fonts(screen, &request.install_root, &mut images, &lay);
    let quads = build_quads(screen, &lay, Vec2::new(win.width(), win.height()), win.scale_factor());
    let n = quads.len().min(SLOTS);
    for (slot, mut node, mut image, mut vis) in slots.iter_mut() {
        let Some(q) = quads.get(slot.0).filter(|_| slot.0 < n) else {
            if slot.0 < *slots_up {
                *vis = Visibility::Hidden;
            }
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
    *slots_up = n;

    let Some(s) = screen.showing.as_mut() else { return };
    s.frames_shown += 1;
    if s.frames_shown == 1 {
        runlog::kv(
            "loading_screen_layout",
            &format!(
                "screen={}x{} right={:.0} loading_top={:.0} map_top={:.0} hint=({:.0},{:.0},w={:.0}) preview=({:.0},{:.0})-({:.0},{:.0}) fonts=[{} {} {}] quads={}",
                physical.x,
                physical.y,
                lay.right,
                lay.loading_top,
                lay.map_top,
                lay.hint_left,
                lay.hint_top,
                lay.hint_width,
                lay.preview.min.x,
                lay.preview.min.y,
                lay.preview.max.x,
                lay.preview.max.y,
                lay.large_font,
                lay.hint_font,
                lay.title_font,
                quads.len()
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
    fn strip_map_drops_folder_and_extension() {
        assert_eq!(strip_map("KF-Farm.rom"), "KF-Farm");
        assert_eq!(strip_map("Maps/KF-Farm.rom"), "KF-Farm");
        assert_eq!(strip_map("C:\\KF\\Maps\\KF-Offices.rom"), "KF-Offices");
        assert_eq!(strip_map("KF-WestLondon"), "KF-WestLondon");
        // The first character is never looked at.
        assert_eq!(strip_map(".rom"), ".rom");
    }

    #[test]
    fn background_pick_retries_up_to_ten_times() {
        let list: Vec<String> = DEFAULT_BACKGROUNDS.iter().map(|s| s.to_string()).collect();
        // Seeded draws: picks 3 first; it loads.
        let mut draws = [3usize, 1].into_iter();
        assert_eq!(pick_background(&list, |_| draws.next().unwrap(), |_| true), Some(3));
        // Nothing loads: ten tries, then none.
        let mut tries = 0;
        assert_eq!(
            pick_background(&list, |n| n - 1, |_| {
                tries += 1;
                false
            }),
            None
        );
        assert_eq!(tries, 10);
        // The second pick loads.
        let mut seen = Vec::new();
        let mut draws = [0usize, 3].into_iter();
        let got = pick_background(&list, |_| draws.next().unwrap(), |p| {
            seen.push(p.to_string());
            p.ends_with('4')
        });
        assert_eq!(got, Some(3));
        assert_eq!(seen, ["2k4Menus.Loading.loadingscreen1", "2k4Menus.Loading.loadingscreen4"]);
        // An empty entry ends the picks (KF keeps MenuBlack).
        assert_eq!(pick_background(&[String::new()], |_| 0, |_| true), None);
        // The seeded generator spreads over the list.
        let mut rng = Rng(0x1234_5678);
        let mut hit = [false; 4];
        for _ in 0..200 {
            hit[rng.rand(4)] = true;
        }
        assert!(hit.iter().all(|&h| h));
    }

    #[test]
    fn hints_parse_from_the_int_array() {
        assert_eq!(quoted_list(r#"("Aim for the head.","Say \"hi\", then run")"#), ["Aim for the head.", "Say \"hi\", then run"]);
        assert!(quoted_list("").is_empty());
    }

    #[test]
    fn layout_follows_the_draw_ops_at_1920_by_1080() {
        let l = layout(Vec2::new(1920.0, 1080.0));
        assert!((l.right - 1900.8).abs() < 1e-3);
        assert!((l.loading_top - 518.4).abs() < 1e-3);
        assert!((l.map_top - 648.0).abs() < 1e-3);
        assert!((l.hint_top - 864.0).abs() < 1e-3);
        // LoadingInfoImage: XS = 1920 / 7 * 3 = 822.86; X = int(960 +
        // (960 - 822.86) / 2) = 1028; Y = int((540 - 462.86) / 5 * 3) = 46.
        assert_eq!((l.preview.min.x, l.preview.min.y), (1028.0, 46.0));
        assert!((l.preview.width() - 822.857).abs() < 1e-2);
        assert!((l.preview.height() - 462.857).abs() < 1e-2);
        assert_eq!(l.title_at, Vec2::new(1032.0, 49.0));
        assert_eq!(l.author_at, Vec2::new(1042.0, 71.0));
        assert_eq!((l.large_font, l.hint_font, l.title_font), ("ROFonts.ROBtsrmVr22", "ROFontsTwo.ROArial22DS", "ROFontsTwo.ROArial18DS"));
        // 800 x 560: sizes [1], the short title font, author 18 below.
        let s = layout(Vec2::new(800.0, 560.0));
        assert_eq!((s.large_font, s.hint_font, s.title_font), ("ROFonts.ROBtsrmVr16", "ROFontsTwo.ROArial14DS", "ROFontsTwo.ROArial22DS"));
        assert_eq!(s.author_at.y - s.title_at.y, 18.0);
    }

    #[test]
    fn font_sizes_step_at_800_1024_1280_1600() {
        let got: Vec<usize> = [640.0, 799.0, 800.0, 1023.0, 1024.0, 1279.0, 1280.0, 1599.0, 1600.0, 2560.0].iter().map(|&w| font_size_index(w)).collect();
        assert_eq!(got, [0, 0, 1, 1, 2, 2, 3, 3, 4, 4]);
    }

    #[test]
    fn hint_lines_wrap_on_words_and_bars() {
        let measure = |s: &str| s.len() as f32;
        assert_eq!(wrap("aaa bbb ccc", 7.0, measure), ["aaa bbb", "ccc"]);
        assert_eq!(wrap("one|two", 100.0, measure), ["one", "two"]);
        assert_eq!(wrap("toolongword x", 3.0, measure), ["toolongword", "x"]);
    }
}
