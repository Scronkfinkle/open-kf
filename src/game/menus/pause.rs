//! The pause menu (KFGui.KFInvasionLoginMenu, a FloatingWindow with a tab
//! control; KFTab_MidGamePerks for the Perks tab). Boxes from KFGui.u
//! and GUI2K4.u; the tab panel is docked under the tab row.
//!
//! Not built: the Communication tab (KFTab_MidGameVoiceChat: player
//! lists, voice options) and the Help tab (KFTab_MidGameHelp: game
//! description, hints); their tabs are drawn and switch, the page shows
//! only the buttons. Settings and Spectate are drawn but do nothing.

use bevy::prelude::*;

use super::gui::{Align, Painter, State, named_font};
use super::{DrawCtx, PauseTab, perk_panel};

/// UT2K4PlayerLoginMenu WinLeft (DefaultLeft) and KFInvasionLoginMenu
/// WinTop / WinWidth / WinHeight.
const WINDOW: [f32; 4] = [0.110313, 0.006158, 0.814844, 0.990311];
/// GUITabControl TabHeight.
const TAB_HEIGHT: f32 = 0.035;

pub(super) fn draw(p: &mut Painter, c: &DrawCtx) {
    let gui = p.gui;
    let screen = p.screen;
    let white = [255, 255, 255, 255];
    let (sw, sh) = (screen.width(), screen.height());
    let win = Rect::new(WINDOW[0] * sw, WINDOW[1] * sh, (WINDOW[0] + WINDOW[2]) * sw, (WINDOW[1] + WINDOW[3]) * sh);
    let wh = win.height();
    // PopupPageBase.FloatingFrameBackground (Thin_border_SlightTransparent,
    // from 4% down).
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Thin_border_SlightTransparent"), Rect::new(win.min.x, win.min.y + 0.04 * wh, win.max.x, win.max.y), white, "FrameBG");
    // FloatingWindow.TitleBar (GUIHeader, Header style = KF_Header:
    // Tabdark; bUseTextHeight: the caption's height plus BorderOffsets 4
    // and 4, a guess), caption = the map (SetTitle: GetURLMap).
    let title_font = named_font("UT2DefaultFont", sw);
    let th = p.line_height(title_font) + 8.0;
    let bar = Rect::new(win.min.x, win.min.y, win.max.x, win.min.y + th);
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Tabdark"), bar, white, "TitleBar");
    p.text_in(title_font, &c.data.map_title, bar, Align::Center, true, [225, 225, 225, 255], "Title");

    // The tab row: LoginMenuTC (TabBackground = ROSTY2TabBackground:
    // Thin_border), buttons KF_TabButton sized to their caption (+44 px,
    // fitted to the screenshot) from 7 px in.
    let tc = gui.comp("KFInvasionLoginMenu.LoginMenuTC").rect(win);
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Thin_border"), tc, white, "TabBackground");
    let menu_font = named_font("UT2MenuFont", sw);
    let tab_h = TAB_HEIGHT * sh;
    let mut x = tc.min.x + 7.0;
    let y = tc.center().y - tab_h / 2.0;
    for (tab, name, id) in [(PauseTab::Perks, "Perks", "pause.tab:perks"), (PauseTab::Communication, "Communication", "pause.tab:communication"), (PauseTab::Help, "Help", "pause.tab:help")] {
        let tw = p.text_size(menu_font, name).x + 44.0;
        let state = if c.state.pause_tab == tab { State::Focused } else { State::Blurry };
        p.button(id, Rect::new(x, y, x + tw, y + tab_h), name, state);
        x += tw + 4.0;
    }
    // The panel docked under the tabs.
    let panel = Rect::new(tc.min.x, tc.min.y + tab_h, tc.max.x, win.max.y);

    if c.state.pause_tab == PauseTab::Perks {
        let row = c.state.pause_row.min(6);
        let bp = gui.comp("KFTab_MidGamePerks.BGPerks");
        p.section(bp.rect(panel), &bp.caption, false, "BGPerks");
        perk_panel::draw_list(p, gui.comp("KFTab_MidGamePerks.PerkSelectList").rect(panel), &c.data.texts, row, c.vet.level, "pause.perk");
        let fx = gui.comp("KFTab_MidGamePerks.BGPerkEffects");
        p.section(fx.rect(panel), &fx.caption, false, "BGPerkEffects");
        perk_panel::draw_effects(p, gui.comp("KFTab_MidGamePerks.PerkEffectsScroll").rect(panel), &c.data.texts, row, c.vet.level);
        let nl = gui.comp("KFTab_MidGamePerks.BGPerksNextLevel");
        p.section(nl.rect(panel), &nl.caption, false, "BGPerksNextLevel");
        perk_panel::draw_requirements(p, gui.comp("KFTab_MidGamePerks.PerkProgressList").rect(panel), &c.data.texts, row);
        let save = gui.comp("KFTab_MidGamePerks.SaveButton");
        p.button("pause.save", save.rect(panel), &save.caption, State::Blurry);
    }

    // SetButtonPositions: the visible buttons (single player: Settings,
    // Spectate, Forfeit, Exit Game) as wide as b_Settings, which
    // autosizes to the longest caption at InitComponent ("Server
    // Browser"); 5% of that apart, centred, at b_Settings' top. The
    // autosize padding (HorzPerc 0.04, VertPerc 0.5) is native: width =
    // text + 0.04 x ResX x 0.25, height = text x 1.5 (fitted to the
    // screenshot, a guess).
    let settings = gui.comp("KFTab_MidGamePerks.SettingsButton");
    let sizing = gui.comp("KFTab_MidGamePerks.BrowserButton").caption;
    let tsz = p.text_size(menu_font, &sizing);
    let bw = tsz.x + 0.04 * sw * 0.25;
    let bh = p.line_height(menu_font) * 1.5;
    let spacing = bw * 0.05;
    // Captions: b_Spec SpectateButtonText, b_Leave LeaveSPButtonText
    // (InitGRI, standalone). b_Spec is enabled once the match has begun
    // (InternalOnPreDraw); ours is drawn inert either way (no spectating).
    let buttons = [("pause.settings", settings.caption.clone()), ("pause.spectate", "Spectate".to_string()), ("pause.forfeit", "Forfeit".to_string()), ("pause.exit", gui.comp("KFTab_MidGamePerks.QuitGameButton").caption)];
    let n = buttons.len() as f32;
    let mut x = panel.center().x - (bw * n + spacing * (n - 1.0)) / 2.0;
    let y = settings.rect(panel).min.y;
    for (id, cap) in buttons {
        p.button(id, Rect::new(x, y, x + bw, y + bh), &cap, State::Blurry);
        x += bw + spacing;
    }
}
