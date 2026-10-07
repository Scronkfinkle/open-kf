//! The character select window (KFGui.KFModelSelect, a GUI2K4
//! UT2k4ModelSelect / LockedFloatingWindow), opened by the perk page's
//! "Change Character". Boxes from KFGui.u and GUI2K4.u, the window's
//! place from the class defaults. See DESIGN.md, "The 3D View and Change
//! Character".
//!
//! Native parts guessed (labelled below): the image list's drawing
//! (GUIVertImageList), the scroll bar's place and look, the alt
//! section's caption place, the title bar's height.

use bevy::prelude::*;

use super::DrawCtx;
use super::gui::{Align, Gui, Painter, State, named_font};

/// KFModelSelect.vil_CharList: NoVisibleCols x NoVisibleRows portraits.
pub const COLS: usize = 4;
pub const ROWS: usize = 3;

/// UT2k4ModelSelect WindowName (GUI2K4.int).
const WINDOW_NAME: &str = "Select Character";

/// The window: the class defaults' WinLeft / WinTop / WinWidth /
/// WinHeight (LockedFloatingWindow: 0.125, 0.15, 0.74, 0.7).
pub(super) fn window(gui: &Gui, screen: Rect) -> Rect {
    let (sw, sh) = (screen.width(), screen.height());
    let l = gui.num("KFGui.KFModelSelect.WinLeft", 0.125);
    let t = gui.num("KFGui.KFModelSelect.WinTop", 0.15);
    let w = gui.num("KFGui.KFModelSelect.WinWidth", 0.74);
    let h = gui.num("KFGui.KFModelSelect.WinHeight", 0.7);
    Rect::new(l * sw, t * sh, (l + w) * sw, (t + h) * sh)
}

/// i_bk, where the SpinnyDude is drawn (bBoundToParent and
/// bScaleToParent: fractions of the window).
pub(super) fn preview_box(gui: &Gui, screen: Rect) -> Rect {
    gui.comp("UT2k4ModelSelect.iBK").rect(window(gui, screen))
}

/// sb_Main: KFModelSelect.InitComponent's SetPosition(0.04, 0.075,
/// 0.680742, 0.555859). The window's own components are only
/// bBoundToParent (LockedFloatingWindow.InternalOnCreateComponent), so the
/// place is a fraction of the window and the size a fraction of the screen.
fn section_box(gui: &Gui, screen: Rect) -> Rect {
    let win = window(gui, screen);
    let c = gui.comp("LockedFloatingWindow.InternalFrameImage");
    // InternalFrameImage's saved values are the same as the SetPosition.
    let [l, t, w, h] = if c.win[2] > 0.0 { c.win } else { [0.04, 0.075, 0.680742, 0.555859] };
    let min = win.min + Vec2::new(l * win.width(), t * win.height());
    Rect::from_corners(min, min + Vec2::new(w * screen.width(), h * screen.height()))
}

/// The list's cells and its scroll bar: sb_Main manages CharList inside
/// its client area (ImageOffset, paddings 0.05 and RightPadding 0.5). The
/// scroll bar (GUIVertScrollBar WinWidth 0.02, of the screen width: a
/// guess) takes the list's right edge; CELL_FixedCount divides the rest
/// into COLS x ROWS.
pub(super) struct ListLayout {
    pub list: Rect,
    pub bar: Rect,
    pub cell: Vec2,
}

pub(super) fn list_layout(gui: &Gui, screen: Rect) -> ListLayout {
    let list = Painter::section_client(section_box(gui, screen), [0.05, 0.05, 0.5, 0.05]);
    let bw = gui.num("XInterface.GUIVertScrollBar.WinWidth", 0.02) * screen.width();
    let bar = Rect::new(list.max.x - bw, list.min.y, list.max.x, list.max.y);
    let cells = Rect::new(list.min.x, list.min.y, bar.min.x, list.max.y);
    ListLayout { list: cells, bar, cell: Vec2::new(cells.width() / COLS as f32, cells.height() / ROWS as f32) }
}

/// The first row that may be shown (the last page is full).
pub(super) fn max_top_row(count: usize) -> usize {
    count.div_ceil(COLS).saturating_sub(ROWS)
}

/// The top row that shows item `i` (SetIndex makes the item visible:
/// GUIVertList, native; we scroll as little as possible).
pub(super) fn top_row_showing(i: usize, top: usize, count: usize) -> usize {
    let row = i / COLS;
    let top = if row < top { row } else if row >= top + ROWS { row + 1 - ROWS } else { top };
    top.min(max_top_row(count))
}

/// LockedFloatingWindow.AlignButtons: OK at the bottom right, Cancel to
/// its left, 10% of a button's size and EdgeBorder in from the edges.
/// The buttons are bBoundToParent only: their size is a fraction of the
/// screen (WinWidth 0.159649, GUIButton WinHeight 0.04).
fn buttons(gui: &Gui, screen: Rect) -> (Rect, Rect) {
    let win = window(gui, screen);
    let (wip, hip) = (win.width(), win.height());
    let ok_c = gui.comp("LockedFloatingWindow.LockedOKButton");
    let cancel_c = gui.comp("LockedFloatingWindow.LockedCancelButton");
    let bh = gui.num("XInterface.GUIButton.WinHeight", 0.04) * screen.height();
    let ok_w = ok_c.win[2] * screen.width();
    let cancel_w = cancel_c.win[2] * screen.width();
    let (xs, ys) = (ok_w * 0.1, bh * 0.1);
    let e2 = gui.num("KFGui.KFModelSelect.EdgeBorder[2]", 16.0);
    let e3 = gui.num("KFGui.KFModelSelect.EdgeBorder[3]", 24.0);
    // Y divides EdgeBorder[3] by the width (WIP), as the script does.
    let x = 1.0 - (ok_w + xs) / wip - e2 / wip;
    let y = 1.0 - (bh + ys) / hip - e3 / wip;
    let ok = Rect::new(win.min.x + x * wip, win.min.y + y * hip, win.min.x + x * wip + ok_w, win.min.y + y * hip + bh);
    let cx = 1.0 - (ok_w + cancel_w + xs) / wip - e2 / wip;
    let cancel = Rect::new(win.min.x + cx * wip, ok.min.y, win.min.x + cx * wip + cancel_w, ok.max.y);
    (ok, cancel)
}

pub(super) fn draw(p: &mut Painter, c: &DrawCtx, preview_texture: Option<usize>) {
    let gui = p.gui;
    let screen = p.screen;
    let white = [255, 255, 255, 255];
    let win = window(gui, screen);
    let chars = &c.data.characters;
    let index = c.state.select_index.min(chars.len().saturating_sub(1));

    // FloatingWindow: the title bar (GUIHeader, KF_Header: Tabdark; the
    // caption's height plus 4 + 4, as the pause menu) and the frame
    // (PopupPageBase.FloatingFrameBackground, Thin_border_SlightTransparent)
    // from the bar's middle down (AlignFrame).
    let title_font = named_font("UT2DefaultFont", screen.width());
    let th = p.line_height(title_font) + 8.0;
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Thin_border_SlightTransparent"), Rect::new(win.min.x, win.min.y + th * 0.5, win.max.x, win.max.y), white, "ModelSelect.FrameBG");
    let bar = Rect::new(win.min.x, win.min.y, win.max.x, win.min.y + th);
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Tabdark"), bar, white, "ModelSelect.TitleBar");
    p.text_in(title_font, WINDOW_NAME, bar, Align::Center, true, [225, 225, 225, 255], "ModelSelect.Title");

    // sb_Main, its caption the highlighted character (ListChange).
    let name = chars.get(index).map_or("", |(n, _)| n.as_str());
    p.alt_section(section_box(gui, screen), name, "ModelSelect.sb_Main");

    // i_bk: changeme_texture at ImageColor alpha 128 (ImageStyle
    // Stretched), its DropShadow 4 px right and down, under it; the model
    // is drawn into the box (InternalOnDraw).
    let bk = preview_box(gui, screen);
    let shadow = gui.tex("InterfaceArt_tex.Menu.changeme_texture");
    p.stretched(shadow, Rect::from_corners(bk.min + 4.0, bk.max + 4.0), [0, 0, 0, 128], "ModelSelect.iBK.Shadow");
    p.stretched(shadow, bk, [255, 255, 255, 128], "ModelSelect.iBK");
    p.hit("select.drag", bk);
    if let Some(t) = preview_texture {
        p.tile(Some(t), bk, white, &format!("ModelSelect.SpinnyDude:{name}"));
    }

    // CharList (GUIVertImageList, native drawing; guessed): each cell the
    // portrait scaled to fit inside the borders (HorzBorder / VertBorder
    // 2 px), centred; the highlighted cell has the ListSelection style's
    // focused image behind it (ROSTY_ListSelection: buttonGreyDark01).
    let lay = list_layout(gui, screen);
    let hb = gui.num("XInterface.GUIVertImageListBox.HorzBorder", 2.0);
    let vb = gui.num("XInterface.GUIVertImageListBox.VertBorder", 2.0);
    let top = c.state.select_top.min(max_top_row(chars.len()));
    for r in 0..ROWS {
        for col in 0..COLS {
            let i = (top + r) * COLS + col;
            let Some((cname, portrait)) = chars.get(i) else { continue };
            let min = lay.list.min + Vec2::new(col as f32 * lay.cell.x, r as f32 * lay.cell.y);
            let cell = Rect::from_corners(min, min + lay.cell);
            if i == index {
                p.stretched(gui.tex("InterfaceArt_tex.Menu.buttonGreyDark01"), cell, white, "ModelSelect.Selection");
            }
            let inner = Rect::new(cell.min.x + hb, cell.min.y + vb, cell.max.x - hb, cell.max.y - vb);
            if let Some(t) = gui.tex(portrait) {
                let size = gui.textures[t].size;
                let s = (inner.width() / size.x).min(inner.height() / size.y);
                let half = size * s * 0.5;
                p.tile(Some(t), Rect::from_center_half_size(inner.center(), half), white, &format!("ModelSelect.Portrait:{cname}"));
            }
            p.hit(&format!("select.cell:{i}"), cell);
        }
    }
    // The scroll bar (guessed look: ROSTY2ScrollZone's solid 32,33,35 zone,
    // the grip and the arrow buttons KF_InterfaceArt_tex.Menu.scrollbar;
    // buttons square; the grip as tall as the visible part of the list).
    let b = lay.bar;
    let bw = b.width();
    p.fill(b, [32, 33, 35, 255], "ModelSelect.ScrollZone");
    let sb_tex = gui.tex("KF_InterfaceArt_tex.Menu.scrollbar");
    let up = Rect::new(b.min.x, b.min.y, b.max.x, b.min.y + bw);
    let down = Rect::new(b.min.x, b.max.y - bw, b.max.x, b.max.y);
    p.stretched(sb_tex, up, white, "ModelSelect.ScrollUp");
    p.stretched(sb_tex, down, white, "ModelSelect.ScrollDown");
    p.hit("select.scroll:-1", up);
    p.hit("select.scroll:1", down);
    let rows = chars.len().div_ceil(COLS).max(1) as f32;
    let zone = Rect::new(b.min.x, up.max.y, b.max.x, down.min.y);
    let gh = (zone.height() * (ROWS as f32 / rows).min(1.0)).max(12.0);
    let max_top = max_top_row(chars.len()).max(1) as f32;
    let gy = zone.min.y + (zone.height() - gh) * (top as f32 / max_top).min(1.0);
    p.stretched(sb_tex, Rect::new(b.min.x, gy, b.max.x, gy + gh), white, "ModelSelect.Grip");

    let (ok, cancel) = buttons(gui, screen);
    p.button("select.cancel", cancel, &gui.comp("LockedFloatingWindow.LockedCancelButton").caption, State::Blurry);
    p.button("select.ok", ok, &gui.comp("LockedFloatingWindow.LockedOKButton").caption, State::Blurry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_scroll_to_show_the_item() {
        // 56 characters: 14 rows, the last top row 11.
        assert_eq!(max_top_row(56), 11);
        assert_eq!(top_row_showing(0, 5, 56), 0);
        assert_eq!(top_row_showing(55, 0, 56), 11);
        assert_eq!(top_row_showing(13, 2, 56), 2);
        assert_eq!(top_row_showing(21, 2, 56), 3);
        assert_eq!(max_top_row(5), 0);
    }
}
