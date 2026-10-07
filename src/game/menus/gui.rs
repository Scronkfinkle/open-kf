//! A small copy of UE2's menu drawing (XInterface / GUI2K4 as KF sets it
//! up): the GUI components' boxes read from `System/KFGui.u`, KF's style
//! textures and fonts, and the native drawing calls the scripts rely on
//! (Canvas.DrawTile, DrawTileStretched, DrawText). Everything is drawn
//! with the HUD's canvas (hud.rs: textured quads, KF's bitmap fonts) in
//! physical pixels. See DESIGN.md, "Menus".

use std::collections::HashMap;

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::PackageSet;
use ue_assets::properties::{Value, read_export_properties};

use crate::engine::runlog;
use crate::game::hud::{Canvas, HudFont, HudTexture, Loader, Quad};

/// One GUI component's saved values (its own, else its class defaults).
#[derive(Clone, Debug, Default)]
pub struct Comp {
    /// WinLeft, WinTop, WinWidth, WinHeight.
    pub win: [f32; 4],
    pub caption: String,
    /// GUILabel TextColor / moCheckBox LabelColor.
    pub color: Option<[u8; 4]>,
    /// GUILabel TextAlign (0 left, 1 centre, 2 right).
    pub align: u8,
    /// GUILabel TextFont.
    pub font: String,
    /// GUIMenuOption CaptionWidth.
    pub caption_width: f32,
}

impl Comp {
    pub fn text_align(&self) -> Align {
        match self.align {
            1 => Align::Center,
            2 => Align::Right,
            _ => Align::Left,
        }
    }

    /// UE2's ActualLeft/Top/Width/Height: a value up to 1 is a fraction of
    /// the parent's size, a larger one is pixels.
    pub fn rect(&self, parent: Rect) -> Rect {
        let f = |v: f32, size: f32| if v.abs() <= 1.0 { v * size } else { v };
        let (w, h) = (parent.width(), parent.height());
        let min = parent.min + Vec2::new(f(self.win[0], w), f(self.win[1], h));
        Rect::from_corners(min, min + Vec2::new(f(self.win[2], w), f(self.win[3], h)))
    }
}

/// Component paths read from KFGui.u (Outer.Name).
pub const COMPONENTS: &[&str] = &[
    "LobbyMenu.ReadyBox0", "LobbyMenu.ReadyBox1", "LobbyMenu.ReadyBox2", "LobbyMenu.ReadyBox3", "LobbyMenu.ReadyBox4", "LobbyMenu.ReadyBox5",
    "LobbyMenu.Player1BackDrop", "LobbyMenu.Player2BackDrop", "LobbyMenu.Player3BackDrop", "LobbyMenu.Player4BackDrop", "LobbyMenu.Player5BackDrop",
    "LobbyMenu.Player6BackDrop", "LobbyMenu.Player1P", "LobbyMenu.Player2P", "LobbyMenu.Player3P", "LobbyMenu.Player4P", "LobbyMenu.Player5P",
    "LobbyMenu.Player6P", "LobbyMenu.Player1Veterancy", "LobbyMenu.Player2Veterancy", "LobbyMenu.Player3Veterancy", "LobbyMenu.Player4Veterancy",
    "LobbyMenu.Player5Veterancy", "LobbyMenu.Player6Veterancy", "LobbyMenu.ChatBox", "LobbyMenu.StoryBoxBackground", "LobbyMenu.GameInfoB",
    "LobbyMenu.CurrentMapL", "LobbyMenu.DifficultyL", "LobbyMenu.WaveB", "LobbyMenu.WaveL", "LobbyMenu.TimeOutCounter", "LobbyMenu.BGPerk",
    "LobbyMenu.BGPerkEffects", "LobbyMenu.PerkEffectsScroll", "LobbyMenu.PlayerPortraitB", "LobbyMenu.PlayerPortrait", "LobbyMenu.ADBG",
    "LobbyMenu.PerkClickArea",
    "LobbyFooter.ReadyButton", "LobbyFooter.Cancel", "LobbyFooter.Options", "LobbyFooter.Perks", "KFLobbyChat.ebSend", "KFPlayerReadyBar.PerkBG",
    "KFPlayerReadyBar.PlayerBG", "KFProfilePage.Panel", "KFTab_Profile.BG3DView", "KFTab_Profile.Player3DView", "KFTab_Profile.bPickModel",
    "KFTab_Profile.DropTarget", "KFTab_Profile.BGPerks", "KFTab_Profile.PerkSelectList", "KFTab_Profile.BGPerkEffects",
    "KFTab_Profile.PerkEffectsScroll", "KFTab_Profile.BGPerksNextLevel", "KFTab_Profile.PerkProgressList", "KFTab_Profile.BGBiography",
    "KFTab_Profile.PlayerPortrait", "KFProfileAndAchievements_Footer.BackB", "KFInvasionLoginMenu.LoginMenuTC", "KFTab_MidGamePerks.BGPerks",
    "KFTab_MidGamePerks.PerkSelectList", "KFTab_MidGamePerks.BGPerkEffects", "KFTab_MidGamePerks.PerkEffectsScroll",
    "KFTab_MidGamePerks.BGPerksNextLevel", "KFTab_MidGamePerks.PerkProgressList", "KFTab_MidGamePerks.SaveButton",
    "KFTab_MidGamePerks.SettingsButton", "KFTab_MidGamePerks.SpectateButton", "KFTab_MidGamePerks.LeaveMatchButton",
    "KFTab_MidGamePerks.QuitGameButton", "KFTab_MidGamePerks.BrowserButton",
];

/// The textures the pages use (styles, list items, the wave circle).
pub const TEXTURES: &[&str] = &[
    "KF_InterfaceArt_tex.Menu.Med_border_SlightTransparent",
    "KF_InterfaceArt_tex.Menu.Thin_border_SlightTransparent",
    "KF_InterfaceArt_tex.Menu.Thin_border",
    "KF_InterfaceArt_tex.Menu.Innerborder",
    "KF_InterfaceArt_tex.Menu.Item_box_box",
    "KF_InterfaceArt_tex.Menu.Item_box_bar",
    "KF_InterfaceArt_tex.Menu.Item_box_box_Highlighted",
    "KF_InterfaceArt_tex.Menu.Item_box_bar_Highlighted",
    "KF_InterfaceArt_tex.Menu.Item_box_box_Disabled",
    "KF_InterfaceArt_tex.Menu.Button",
    "KF_InterfaceArt_tex.Menu.button_Highlight",
    "KF_InterfaceArt_tex.Menu.button_pressed",
    "KF_InterfaceArt_tex.Menu.Tabdark",
    "KF_InterfaceArt_tex.Menu.Checkbox",
    "InterfaceArt_tex.Menu.progress_bar",
    "KillingFloorHUD.HUD.Hud_Bio_Circle",
];

/// The fonts (GUI2K4.int fntUT2k4Small / Menu / Default, ROEngine.int
/// ROHud MenuFontArrayNames).
pub const FONTS: &[&str] = &[
    "ROFonts.ROBtsrmVr7", "ROFonts.ROBtsrmVr8", "ROFonts.ROBtsrmVr9", "ROFonts.ROBtsrmVr10", "ROFonts.ROBtsrmVr12", "ROFonts.ROBtsrmVr14",
    "ROFonts.ROBtsrmVr16", "ROFonts.ROBtsrmVr18",
];

/// GUI2K4.int: fntUT2k4Small (KeyName UT2SmallFont) and fntUT2k4Menu
/// (UT2MenuFont); fntUT2k4Default (UT2DefaultFont) has one size.
const UT2_SMALL: [&str; 5] = ["ROFonts.ROBtsrmVr7", "ROFonts.ROBtsrmVr8", "ROFonts.ROBtsrmVr10", "ROFonts.ROBtsrmVr12", "ROFonts.ROBtsrmVr14"];
const UT2_MENU: [&str; 5] = ["ROFonts.ROBtsrmVr8", "ROFonts.ROBtsrmVr10", "ROFonts.ROBtsrmVr12", "ROFonts.ROBtsrmVr14", "ROFonts.ROBtsrmVr16"];
const UT2_DEFAULT: &str = "ROFonts.ROBtsrmVr10";

/// GUIFont.GetFont(XRes) is native: assumed one size step per resolution
/// class (under 640, 800, 1024, 1280, then the largest). A guess, checked
/// against the screenshots (2560 wide: Vr16 for UT2MenuFont).
pub fn gui_font_index(width: f32) -> usize {
    if width < 640.0 {
        0
    } else if width < 800.0 {
        1
    } else if width < 1024.0 {
        2
    } else if width < 1280.0 {
        3
    } else {
        4
    }
}

/// ROHUD.GetSmallMenuFont (KFPerkSelectList, KFPerkProgressList,
/// LobbyMenu.DrawPerk): MenuFontArrayNames by ClipX.
pub fn small_menu_font(width: f32) -> &'static str {
    if width < 800.0 {
        "ROFonts.ROBtsrmVr7"
    } else if width < 1024.0 {
        "ROFonts.ROBtsrmVr9"
    } else if width < 1280.0 {
        "ROFonts.ROBtsrmVr12"
    } else if width < 1600.0 {
        "ROFonts.ROBtsrmVr14"
    } else {
        "ROFonts.ROBtsrmVr18"
    }
}

/// The font a component's TextFont / style FontNames name means.
pub fn named_font(name: &str, width: f32) -> &'static str {
    let i = gui_font_index(width);
    match name.to_ascii_lowercase().as_str() {
        "ut2smallfont" => UT2_SMALL[i],
        "ut2defaultfont" => UT2_DEFAULT,
        _ => UT2_MENU[i],
    }
}

/// Everything the menus load once: component values, textures, fonts and
/// texts.
#[derive(Resource, Default)]
pub struct Gui {
    pub comps: HashMap<String, Comp>,
    pub textures: Vec<HudTexture>,
    tex_by_path: HashMap<String, Option<usize>>,
    pub fonts: Vec<HudFont>,
    font_by_name: HashMap<String, usize>,
    /// A 1 x 1 white texture (solid fills: the video panel).
    pub white: Option<usize>,
    pub loaded: bool,
}

impl Gui {
    pub fn comp(&self, path: &str) -> Comp {
        self.comps.get(path).cloned().unwrap_or_default()
    }

    pub fn tex(&self, path: &str) -> Option<usize> {
        self.tex_by_path.get(&path.to_ascii_lowercase()).copied().flatten()
    }

    pub fn font(&self, name: &str) -> Option<&HudFont> {
        self.font_by_name.get(&name.to_ascii_lowercase()).map(|&i| &self.fonts[i])
    }
}

/// Loads the component values, textures and fonts. `extra` are more
/// texture paths (perk icons, portraits).
pub fn load(gui: &mut Gui, root: &std::path::Path, extra: &[String], images: &mut Assets<Image>) {
    let set = PackageSet::new(root);
    let defaults = ClassDefaults::new(&set);
    let mut loader = Loader { set: &set, images, textures: Vec::new(), by_path: HashMap::new(), missing: Vec::new() };
    let mut missing_comps = Vec::new();
    // System/KFGui.u by its file (the name "KFGui" also fits
    // Textures/KFGui.utx), its exports by path.
    let kfgui = set.load_path(&root.join("System").join("KFGui.u")).ok();
    let by_path: HashMap<String, usize> = kfgui
        .as_ref()
        .map(|lp| (0..lp.pkg.exports.len()).map(|i| (lp.pkg.object_path(ue_assets::package::ObjectRef::Export(i)).to_ascii_lowercase(), i)).collect())
        .unwrap_or_default();
    for path in COMPONENTS {
        let Some(h) = kfgui.as_ref().zip(by_path.get(&path.to_ascii_lowercase())).map(|(lp, &export)| ue_assets::package_set::ObjectHandle { package: lp.clone(), export }) else {
            missing_comps.push(format!("{path}:not_found"));
            continue;
        };
        let own = match read_export_properties(&h.package.pkg, h.export) {
            Ok(o) => o,
            Err(e) => {
                missing_comps.push(format!("{path}:{e}"));
                continue;
            }
        };
        let v = |n: &str| defaults.actor_value(&h.package, h.export, &own, n);
        let f = |n: &str| match v(n) {
            Some(Value::Float(x)) => x,
            Some(Value::Int(x)) => x as f32,
            _ => 0.0,
        };
        let color = match v("TextColor").or_else(|| v("LabelColor")) {
            Some(Value::Color(c)) => Some(c),
            _ => None,
        };
        let comp = Comp {
            win: [f("WinLeft"), f("WinTop"), f("WinWidth"), f("WinHeight")],
            caption: match v("Caption") {
                Some(Value::Str(s)) => s,
                _ => String::new(),
            },
            color,
            align: match v("TextAlign") {
                Some(Value::Byte(b)) => b,
                _ => 0,
            },
            font: match v("TextFont").or_else(|| v("LabelFont")) {
                Some(Value::Str(s)) => s,
                _ => String::new(),
            },
            caption_width: f("CaptionWidth"),
        };
        gui.comps.insert(path.to_string(), comp);
    }
    for path in TEXTURES.iter().map(|s| s.to_string()).chain(extra.iter().cloned()) {
        let t = loader.texture_path(&path);
        gui.tex_by_path.insert(path.to_ascii_lowercase(), t);
    }
    let mut fonts_missing = Vec::new();
    for name in FONTS {
        let Some(h) = set.find_object(name, Some("Font")) else {
            fonts_missing.push(name.to_string());
            continue;
        };
        match ue_assets::font::read_font(&h.package.pkg, h.export) {
            Ok(font) => {
                let pages = font.textures.iter().map(|&t| loader.texture(&h.package, t)).collect();
                gui.font_by_name.insert(name.to_ascii_lowercase(), gui.fonts.len());
                gui.fonts.push(HudFont { name: name.to_string(), font, pages });
            }
            Err(e) => fonts_missing.push(format!("{name}:{e}")),
        }
    }
    let mut textures = loader.textures;
    let missing = loader.missing;
    let white = Image::new_fill(
        bevy::render::render_resource::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        &[255, 255, 255, 255],
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    textures.push(HudTexture { image: images.add(white), size: Vec2::ONE });
    gui.white = Some(textures.len() - 1);
    gui.textures = textures;
    gui.loaded = true;
    runlog::kv(
        "menu_layout",
        &format!(
            "components={} missing_components=[{}] textures={} missing_textures=[{}] fonts={} missing_fonts=[{}] {}",
            gui.comps.len(),
            missing_comps.join(" "),
            gui.textures.len(),
            missing.join(" "),
            gui.fonts.len(),
            fonts_missing.join(" "),
            {
                let mut v: Vec<String> = gui.comps.iter().map(|(k, c)| format!("{k}:{:?}", c.win)).collect();
                v.sort();
                v.join(" ")
            }
        ),
    );
}

/// Horizontal text alignment (eTextAlign).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A button's look (MenuState: 0 blurry, 1 watched, 2 focused, 3 pressed,
/// 4 disabled).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Blurry,
    Watched,
    Focused,
    Disabled,
}

/// One frame of menu drawing, in physical pixels.
pub struct Painter<'a> {
    pub gui: &'a Gui,
    pub canvas: Canvas,
    /// The screen in physical pixels.
    pub screen: Rect,
    /// The mouse, physical pixels.
    pub mouse: Option<Vec2>,
    /// Clickable boxes drawn this frame: (id, box).
    pub hits: Vec<(String, Rect)>,
}

impl<'a> Painter<'a> {
    pub fn new(gui: &'a Gui, size_logical: Vec2, scale_factor: f32, mouse: Option<Vec2>) -> Self {
        let mut canvas = Canvas::new(size_logical, 255);
        canvas.scale_factor = scale_factor;
        let screen = Rect::from_corners(Vec2::ZERO, size_logical * scale_factor);
        Painter { gui, canvas, screen, mouse, hits: Vec::new() }
    }

    pub fn width(&self) -> f32 {
        self.screen.width()
    }

    fn push(&mut self, texture: usize, uv: Rect, rect: Rect, tint: [u8; 4], what: &str) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let sf = self.canvas.scale_factor;
        self.canvas.quads.push(Quad {
            texture,
            uv,
            screen: Rect::from_corners(rect.min / sf, rect.max / sf),
            tint,
            what: what.to_string(),
        });
    }

    /// Canvas.DrawTile of the whole texture.
    pub fn tile(&mut self, texture: Option<usize>, rect: Rect, tint: [u8; 4], what: &str) {
        let Some(t) = texture else { return };
        let size = self.gui.textures[t].size;
        self.push(t, Rect::from_corners(Vec2::ZERO, size), rect, tint, what);
    }

    /// A solid box (the white texture tinted).
    pub fn fill(&mut self, rect: Rect, color: [u8; 4], what: &str) {
        if let Some(w) = self.gui.white {
            self.push(w, Rect::new(0.0, 0.0, 1.0, 1.0), rect, color, what);
        }
    }

    /// Canvas.DrawTileStretched (native; assumed UE2's rule): the
    /// texture's four quarters keep their size at the corners (scaled down
    /// when the box is smaller than the texture), the middle row and
    /// column of texels stretch along the edges and over the middle.
    pub fn stretched(&mut self, texture: Option<usize>, rect: Rect, tint: [u8; 4], what: &str) {
        let Some(t) = texture else { return };
        let size = self.gui.textures[t].size;
        let mid = (size / 2.0).floor();
        let corner = Vec2::new(mid.x.min(rect.width() / 2.0), mid.y.min(rect.height() / 2.0));
        // Columns: (screen x0, x1, texel u0, u1).
        let xs = [
            (rect.min.x, rect.min.x + corner.x, 0.0, mid.x),
            (rect.min.x + corner.x, rect.max.x - corner.x, mid.x - 0.5, mid.x + 0.5),
            (rect.max.x - corner.x, rect.max.x, size.x - mid.x, size.x),
        ];
        let ys = [
            (rect.min.y, rect.min.y + corner.y, 0.0, mid.y),
            (rect.min.y + corner.y, rect.max.y - corner.y, mid.y - 0.5, mid.y + 0.5),
            (rect.max.y - corner.y, rect.max.y, size.y - mid.y, size.y),
        ];
        for (yi, y) in ys.iter().enumerate() {
            for (xi, x) in xs.iter().enumerate() {
                if x.1 - x.0 < 0.01 || y.1 - y.0 < 0.01 {
                    continue;
                }
                self.push(t, Rect::new(x.2, y.2, x.3, y.3), Rect::new(x.0, y.0, x.1, y.1), tint, &format!("{what}[{yi}{xi}]"));
            }
        }
    }

    pub fn text_size(&self, font: &str, text: &str) -> Vec2 {
        self.gui.font(font).map_or(Vec2::ZERO, |f| Canvas::text_size(f, text, 1.0))
    }

    /// The height of a line of this font (the tallest glyph of "Wq").
    pub fn line_height(&self, font: &str) -> f32 {
        self.text_size(font, "Wqg|").y
    }

    /// Canvas.DrawText at a pixel position.
    pub fn text(&mut self, font: &str, text: &str, at: Vec2, color: [u8; 4], what: &str) {
        if let Some(f) = self.gui.font(font) {
            self.canvas.text(f, text, at, 1.0, color, what);
        }
    }

    /// Text placed in a box: horizontal alignment, centred vertically if
    /// `vcenter`, else at the top.
    #[allow(clippy::too_many_arguments)]
    pub fn text_in(&mut self, font: &str, text: &str, rect: Rect, align: Align, vcenter: bool, color: [u8; 4], what: &str) {
        let size = self.text_size(font, text);
        let x = match align {
            Align::Left => rect.min.x,
            Align::Center => rect.center().x - size.x / 2.0,
            Align::Right => rect.max.x - size.x,
        };
        let y = if vcenter { rect.center().y - self.line_height(font) / 2.0 } else { rect.min.y };
        self.text(font, text, Vec2::new(x.round(), y.round()), color, what);
    }

    /// Canvas.WrapStringToArray: words onto lines no wider than `width`;
    /// `|` starts a new line (GUIScrollText's Separator).
    pub fn wrap(&self, font: &str, text: &str, width: f32) -> Vec<String> {
        let mut out = Vec::new();
        for para in text.split('|') {
            let mut line = String::new();
            for word in para.split(' ') {
                let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if !line.is_empty() && self.text_size(font, &candidate).x > width {
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

    /// A GUIScrollTextBox's text (no scrolling: the lines that fit).
    pub fn scroll_text(&mut self, font: &str, text: &str, rect: Rect, color: [u8; 4], what: &str) {
        let h = self.line_height(font);
        let mut y = rect.min.y;
        for line in self.wrap(font, text, rect.width()) {
            if y + h > rect.max.y + 0.5 {
                break;
            }
            self.text(font, &line, Vec2::new(rect.min.x, y), color, what);
            y += h;
        }
    }

    /// GUISectionBackground (native drawing): HeaderBase stretched over
    /// the box (Med_border_SlightTransparent; AltSectionBackground:
    /// Thin_border_SlightTransparent), the caption in the TextLabel style
    /// (KF_TextLabel: 200,200,200,200), small font, in the header strip
    /// (the texture's top 24 texels). Caption place fitted to the
    /// screenshots: 22 px in, centred in the strip.
    pub fn section(&mut self, rect: Rect, caption: &str, alt: bool, what: &str) {
        let tex = if alt {
            self.gui.tex("KF_InterfaceArt_tex.Menu.Thin_border_SlightTransparent")
        } else {
            self.gui.tex("KF_InterfaceArt_tex.Menu.Med_border_SlightTransparent")
        };
        self.stretched(tex, rect, [255, 255, 255, 255], what);
        if !caption.is_empty() {
            let font = named_font("UT2SmallFont", self.width());
            let strip = Rect::new(rect.min.x + 22.0, rect.min.y, rect.max.x, rect.min.y + 24.0);
            self.text_in(font, caption, strip, Align::Left, true, [200, 200, 200, 200], &format!("{what}.Caption"));
        }
    }

    /// The client area of a section, where its managed components go:
    /// inside ImageOffset (20, 35, 10, 10 px; GUISectionBackground
    /// defaults) and the paddings (fractions of the box).
    pub fn section_client(rect: Rect, pad: [f32; 4]) -> Rect {
        let inner = Rect::new(rect.min.x + 20.0, rect.min.y + 35.0, rect.max.x - 10.0, rect.max.y - 10.0);
        let (w, h) = (inner.width(), inner.height());
        Rect::new(inner.min.x + pad[0] * w, inner.min.y + pad[1] * h, inner.max.x - pad[2] * w, inner.max.y - pad[3] * h)
    }

    /// Is the mouse over this box?
    pub fn hover(&self, rect: Rect) -> bool {
        self.mouse.is_some_and(|m| rect.contains(m))
    }

    /// A GUIButton in a KF button style (KF_SquareButton, KF_FooterButton,
    /// KF_TabButton: Button / button_Highlight / button_pressed, black
    /// text, white when focused). Records the box for clicks.
    pub fn button(&mut self, id: &str, rect: Rect, caption: &str, state: State) {
        let state = if state == State::Blurry && self.hover(rect) { State::Watched } else { state };
        let (tex, color) = match state {
            State::Blurry | State::Disabled => ("KF_InterfaceArt_tex.Menu.Button", [0, 0, 0, 255]),
            State::Watched => ("KF_InterfaceArt_tex.Menu.button_Highlight", [0, 0, 0, 255]),
            State::Focused => ("KF_InterfaceArt_tex.Menu.button_Highlight", [255, 255, 255, 255]),
        };
        let t = self.gui.tex(tex);
        self.stretched(t, rect, [255, 255, 255, 255], id);
        let font = named_font("UT2MenuFont", self.width());
        self.text_in(font, caption, rect, Align::Center, true, color, &format!("{id}.Caption"));
        if state != State::Disabled {
            self.hits.push((id.to_string(), rect));
        }
    }

    /// A box that reacts to clicks without drawing anything.
    pub fn hit(&mut self, id: &str, rect: Rect) {
        self.hits.push((id.to_string(), rect));
    }
}

/// UI image nodes reused every frame for the menus (glyphs count one each).
pub const POOL: usize = 3000;

#[derive(Component)]
pub struct MenuSlot(pub usize);

/// Copies the quads to the node pool (as hud.rs does); `shown` is how many
/// slots were visible last frame, so only those are touched again.
pub fn flush(
    quads: &[Quad],
    gui: &Gui,
    slots: &mut Query<(&MenuSlot, &mut Node, &mut ImageNode, &mut Visibility)>,
    shown: &mut usize,
) {
    let n = quads.len().min(POOL);
    if quads.len() > POOL {
        runlog::kv("menu_pool_full", &format!("quads={} pool={POOL}", quads.len()));
    }
    let touch = n.max(*shown);
    for (slot, mut node, mut image, mut vis) in slots.iter_mut() {
        if slot.0 >= touch {
            continue;
        }
        let Some(q) = quads.get(slot.0) else {
            *vis = Visibility::Hidden;
            continue;
        };
        node.left = Val::Px(q.screen.min.x);
        node.top = Val::Px(q.screen.min.y);
        node.width = Val::Px(q.screen.width());
        node.height = Val::Px(q.screen.height());
        let t = &gui.textures[q.texture];
        if image.image != t.image {
            image.image = t.image.clone();
        }
        image.rect = Some(q.uv);
        image.color = Color::srgba_u8(q.tint[0], q.tint[1], q.tint[2], q.tint[3]);
        *vis = Visibility::Inherited;
    }
    *shown = n;
}

/// Reads a localization file (Latin-1, as KF writes them).
pub fn read_latin1(path: &std::path::Path) -> String {
    std::fs::read(path).map(|b| b.iter().map(|&c| c as char).collect()).unwrap_or_default()
}

/// A value of an .int / .ini file: `[section] key=` (quotes removed).
pub fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    crate::audio::music::int_section(text, section)
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| {
            // One pair of quotes (a value may itself start with a quote:
            // Mr_Foster's biography).
            let v = v.trim();
            let v = v.strip_prefix('"').unwrap_or(v);
            v.strip_suffix('"').unwrap_or(v).to_string()
        })
}
