//! The perk widget shared by the perk page (KFTab_Profile) and the pause
//! menu's Perks tab (KFTab_MidGamePerks): the list of the seven perks
//! (KFPerkSelectList.DrawPerk), "Perk Effects" (LevelEffects of the
//! row's level, a GUIScrollTextBox) and "Next Level Requirements"
//! (KFPerkProgressList.DrawPerk). Also the lobby's "Current Perk" box
//! (LobbyMenu.DrawPerk), which draws one row the same way.
//!
//! Levels are chosen, not earned (perks stage 2 is not built) and KF's
//! thresholds are native (KFSteamStatsAndAchievements): every progress bar
//! is empty, `%x` in a requirement shows "?", and the progress text reads
//! "not tracked yet" (ours) where KF prints e.g. "14002/25000".

use bevy::prelude::*;

use super::gui::{Painter, named_font, small_menu_font};
use crate::game::perks::Perk;

/// KFMod.int texts of each perk (PerkIndex order).
#[derive(Clone, Debug, Default)]
pub struct PerkTexts {
    pub names: [String; 7],
    /// LevelEffects[0..6].
    pub effects: [[String; 7]; 7],
    pub requirements: [Vec<String>; 7],
}

impl PerkTexts {
    pub fn load(root: &std::path::Path) -> Self {
        let text = super::gui::read_latin1(&root.join("System").join("KFMod.int"));
        let mut t = PerkTexts::default();
        for p in Perk::ALL {
            let i = p.index();
            let get = |k: &str| super::gui::ini_value(&text, p.class(), k);
            t.names[i] = get("VeterancyName").unwrap_or_else(|| p.name().to_string());
            for l in 0..7 {
                t.effects[i][l] = get(&format!("LevelEffects[{l}]")).unwrap_or_default();
            }
            t.requirements[i] = (0..8).map_while(|r| get(&format!("Requirements[{r}]"))).collect();
        }
        t
    }
}

/// The settings of KFPerkSelectList / LobbyMenu (class defaults: the
/// same numbers in both).
const ICON_BORDER: f32 = 0.05;
const ITEM_BORDER: f32 = 0.11;
const TEXT_TOP_OFFSET: f32 = 0.05;
const ICON_TO_INFO_SPACING: f32 = 0.05;
const PROGRESS_BAR_HEIGHT: f32 = 0.3;
/// KFPerkProgressList defaults.
const REQ_ITEM_BORDER: f32 = 0.018;
const REQ_TEXT_TOP_OFFSET: f32 = -0.14;

/// Our progress: none tracked (see the module comment).
const PERK_PROGRESS: f32 = 0.0;

/// One perk row. `lobby`: LobbyMenu.DrawPerk's version (the bar sits
/// TempHeight + 0.04 H below the name instead of TempHeight - 0.04 H).
#[allow(clippy::too_many_arguments)]
pub fn draw_row(p: &mut Painter, at: Vec2, width: f32, height: f32, perk: Perk, level: u8, name: &str, selected: bool, hovered: bool, lobby: bool) {
    let (x, y) = (at.x, at.y);
    // ItemSpacing is 0 (not set in the defaults).
    let mut icon = height;
    let (box_tex, bar_tex) = if selected {
        ("KF_InterfaceArt_tex.Menu.Item_box_box_Highlighted", "KF_InterfaceArt_tex.Menu.Item_box_bar_Highlighted")
    } else {
        ("KF_InterfaceArt_tex.Menu.Item_box_box", "KF_InterfaceArt_tex.Menu.Item_box_bar")
    };
    let white = [255, 255, 255, 255];
    p.stretched(p.gui.tex(box_tex), Rect::new(x, y, x + icon, y + icon), white, &format!("PerkBox:{}", perk.class()));
    p.stretched(p.gui.tex(bar_tex), Rect::new(x + icon - 1.0, y + 7.0, x + width - 1.0, y + height - 7.0), white, &format!("PerkBar:{}", perk.class()));
    icon -= ICON_BORDER * 2.0 * height;
    let icon_tex = p.gui.tex(perk.icons().0);
    let ix = x + ICON_BORDER * height;
    p.tile(icon_tex, Rect::new(ix, y + ICON_BORDER * height, ix + icon, y + ICON_BORDER * height + icon), white, &format!("PerkIcon:{}", perk.class()));
    let tx = x + icon + ICON_TO_INFO_SPACING * width;
    let mut ty = y + TEXT_TOP_OFFSET * height + ITEM_BORDER * height;
    let bar_w = width - (tx - x) - ICON_TO_INFO_SPACING * width;
    let font = small_menu_font(p.width());
    // MouseOverIndex: red, else black.
    let color = if hovered { [255, 0, 0, 255] } else { [0, 0, 0, 255] };
    p.text(font, name, Vec2::new(tx, ty).round(), color, &format!("PerkName:{}", perk.class()));
    let lv = format!("Lv {level}");
    let size = p.text_size(font, &lv);
    p.text(font, &lv, Vec2::new(tx + bar_w - size.x, ty).round(), color, &format!("PerkLevel:{}", perk.class()));
    let th = p.text_size(font, name).y.max(size.y);
    ty += if lobby { th + 0.04 * height } else { th - 0.04 * height };
    let bh = PROGRESS_BAR_HEIGHT * height;
    p.stretched(p.gui.tex("KF_InterfaceArt_tex.Menu.Innerborder"), Rect::new(tx, ty, tx + bar_w, ty + bh), white, "PerkProgressBG");
    p.stretched(
        p.gui.tex("InterfaceArt_tex.Menu.progress_bar"),
        Rect::new(tx + 3.0, ty + 3.0, tx + 3.0 + (bar_w - 6.0) * PERK_PROGRESS, ty + bh - 3.0),
        white,
        "PerkProgress",
    );
}

/// KFPerkSelectList: seven rows of MenuOwner height / 7 - 1. Rows are
/// clickable (`{id}:N`).
pub fn draw_list(p: &mut Painter, list: Rect, texts: &PerkTexts, selected: usize, level: u8, id: &str) {
    let h = list.height() / 7.0 - 1.0;
    for perk in Perk::ALL {
        let i = perk.index();
        let at = Vec2::new(list.min.x, list.min.y + i as f32 * h);
        let row = Rect::from_corners(at, at + Vec2::new(list.width(), h));
        let hovered = p.hover(row);
        draw_row(p, at, list.width(), h, perk, level, &texts.names[i], i == selected, hovered, false);
        p.hit(&format!("{id}:{i}"), row);
    }
}

/// "Perk Effects": LevelEffects[level] in a GUIScrollTextBox (FontScale
/// medium: UT2MenuFont; NoBackground style colour 225,225,225).
pub fn draw_effects(p: &mut Painter, rect: Rect, texts: &PerkTexts, perk: usize, level: u8) {
    let font = named_font("UT2MenuFont", p.width());
    let text = texts.effects[perk][level.min(6) as usize].clone();
    p.scroll_text(font, &text, rect, [225, 225, 225, 255], "PerkEffects");
}

/// "Next Level Requirements" (KFPerkProgressList.DrawPerk), one item per
/// requirement of MenuOwner height / 3 - 1. Ours: no numbers (see the
/// module comment).
pub fn draw_requirements(p: &mut Painter, list: Rect, texts: &PerkTexts, perk: usize) {
    let font = small_menu_font(p.width());
    let h = list.height() / 3.0 - 1.0;
    let aspect = p.screen.width() / p.screen.height();
    let white = [255, 255, 255, 255];
    for (k, req) in texts.requirements[perk].iter().enumerate() {
        let (x, y, w) = (list.min.x, list.min.y + k as f32 * h, list.width());
        let border = (3.0 - aspect) * REQ_ITEM_BORDER * w;
        p.stretched(p.gui.tex("KF_InterfaceArt_tex.Menu.Thin_border"), Rect::new(x, y, x + w, y + h), white, &format!("Requirement{k}"));
        let tx = x + border;
        let ty = y + (3.0 - aspect) * border + REQ_TEXT_TOP_OFFSET * h;
        // Repl(Requirements[i], "%x", FormatNumber(Denominator)): the
        // threshold is native, unknown here.
        let text = req.replace("%x", "?");
        let lines = p.wrap(font, &text, w - border * 2.0);
        let lh = p.text_size(font, lines.first().map_or("", |s| s.as_str())).y;
        if let Some(l) = lines.first() {
            p.text(font, l, Vec2::new(tx, ty).round(), [192, 192, 192, 255], &format!("RequirementText{k}"));
        }
        if let Some(l) = lines.get(1) {
            p.text(font, l, Vec2::new(tx, ty + lh * 0.8).round(), [192, 192, 192, 255], &format!("RequirementText{k}"));
        }
        // KF: Numerator/Denominator. Ours: not tracked.
        let progress = "not tracked yet";
        let ps = p.text_size(font, progress);
        let px = w - ps.x - border;
        let py = y + h - ps.y;
        p.text(font, progress, Vec2::new(x + px + 2.0, py - 2.0).round(), [192, 192, 192, 255], &format!("RequirementProgress{k}"));
        let bar_h = h - (py - y) - border / 2.0;
        p.stretched(p.gui.tex("KF_InterfaceArt_tex.Menu.Innerborder"), Rect::new(x + border, py, x + px, py + bar_h), white, &format!("RequirementBar{k}"));
    }
}
