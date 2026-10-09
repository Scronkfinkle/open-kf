//! The map vote window (KFGui.KFMapVotingPage = ROInterface.ROMapVotingPage
//! on xVoting.MapVotingPage, a LargeWindow): the vote count list on top
//! (maps with votes, most first), the map list under it ("Map Name",
//! "Played", "Seq"), the footer with Submit and Close. Boxes from
//! XVoting.u / ROInterface.u, texts from XVoting.int.
//!
//! KF's list drawing is native (GUIMultiColumnList with the
//! ServerBrowserGrid style); row height, column widths and colours here
//! are guesses. KF's footer is a chat box; ours prints the vote's state
//! there instead (time left, my vote, the winner) and has no chat.
//! KF shows no countdown in the window (the announcer says it); the
//! "Voting ends in" line is ours.
//!
//! A row is picked by clicking it; Submit votes for it (KF: Submit, or a
//! double click). The mouse wheel scrolls the map list.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use super::gui::{Align, Painter, State, named_font};
use super::{MenuState, Page};
use crate::engine::runlog;
use crate::game::map_vote::rules::{VoteView, won_text};
use crate::game::map_vote::{CastMapVote, MapVote};

/// VotingPage WinLeft, WinTop, WinWidth, WinHeight.
const WINDOW: [f32; 4] = [0.1, 0.1, 0.8, 0.8];
/// MapVotingPage.WindowName and lmsgMode[0] (majority mode).
const TITLE: &str = "Map Voting (Majority Mode)";

/// What the window needs besides the menu state.
pub(crate) struct VoteDraw<'a> {
    pub view: &'a VoteView,
    /// My peer id.
    pub me: u64,
    /// Peer -> player name.
    pub names: Vec<(u64, String)>,
}

/// Opens the window when the vote's windows open (OpenAllVoteWindows),
/// closes it when the vote is over or the player closed it. The mouse is
/// freed while it is up (LargeWindow bCaptureInput).
pub(super) fn sync_page(
    time: Res<Time<Real>>,
    vote: Res<MapVote>,
    mut state: ResMut<MenuState>,
    mut window: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let wanted = vote.window_wanted(time.elapsed_secs());
    let open = state.stack.contains(&Page::MapVote);
    if wanted == open {
        return;
    }
    let Ok(mut cursor) = window.single_mut() else { return };
    if wanted {
        // Above everything else (the lobby, the pause menu).
        state.vote_cursor_was_grabbed = cursor.grab_mode != CursorGrabMode::None;
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
        state.stack.push(Page::MapVote);
        state.vote_row = None;
        state.vote_top = 0;
        let v = vote.view.as_ref();
        runlog::kv("menu_open", &format!("page=MapVote maps={} time_left={}", v.map_or(0, |v| v.maps.len()), v.map_or(-1, |v| v.time_left)));
    } else {
        state.stack.retain(|p| *p != Page::MapVote);
        if state.stack.is_empty() && state.vote_cursor_was_grabbed {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
        let why = if vote.closed_by_player {
            "closed"
        } else if vote.won_at.is_some() {
            "map_won"
        } else {
            "vote_over"
        };
        runlog::kv("menu_close", &format!("page=MapVote reason={why}"));
    }
}

/// Test actions (`--input FRAME:ACTION`) for the window.
pub(super) fn scripted_ids(action: &str) -> Option<String> {
    match action {
        "vote_submit" => Some("vote.submit".into()),
        "vote_close" => Some("vote.close".into()),
        "vote_open" => Some("vote.open".into()),
        a => {
            if let Some(n) = a.strip_prefix("vote:") {
                Some(format!("vote.pick:{n}"))
            } else if let Some(n) = a.strip_prefix("vote_row:") {
                Some(format!("vote.row:{n}"))
            } else {
                a.strip_prefix("vote_scroll:").map(|n| format!("vote.scroll:{n}"))
            }
        }
    }
}

/// A click or a test action on the window. Returns false when the id is
/// not the window's.
pub(super) fn apply(id: &str, state: &mut MenuState, vote: &mut MapVote, cast: &mut MessageWriter<CastMapVote>) -> bool {
    let Some(rest) = id.strip_prefix("vote.") else { return false };
    let on = state.top() == Some(Page::MapVote);
    let maps: Vec<(String, bool)> = vote.view.as_ref().map(|v| v.maps.iter().map(|m| (m.name.clone(), m.enabled)).collect()).unwrap_or_default();
    // Submit / a pick: the selected map, checked as MapVotingPage.SendVote
    // (a disabled map: lmsgMapDisabled, nothing sent).
    let mut submit = |row: Option<usize>, how: &str| match row.and_then(|r| maps.get(r)) {
        Some((name, true)) => {
            runlog::kv("map_vote_submit", &format!("map={name} via={how}"));
            cast.write(CastMapVote { map: name.clone() });
        }
        Some((name, false)) => runlog::kv("map_vote_submit", &format!("map={name} via={how} refused=\"The selected Map is disabled.\"")),
        None => runlog::kv("map_vote_submit", &format!("via={how} refused=no_map_selected")),
    };
    match rest {
        "submit" if on => submit(state.vote_row, "submit"),
        "close" if on => {
            vote.closed_by_player = true;
            runlog::kv("map_vote_window", "event=close_button");
        }
        // KF reopens it from the mid-game menu ("MAP VOTING"); ours only by
        // the test action.
        "open" => {
            vote.closed_by_player = false;
            runlog::kv("map_vote_window", &format!("event=reopen vote={}", vote.view.is_some()));
        }
        r => {
            if let Some(n) = r.strip_prefix("row:").and_then(|n| n.parse::<usize>().ok()).filter(|_| on) {
                if n < maps.len() {
                    state.vote_row = Some(n);
                    runlog::kv("map_vote_select", &format!("row={n} map={}", maps[n].0));
                }
            } else if let Some(name) = r.strip_prefix("pick:").filter(|_| on) {
                let row = maps.iter().position(|(m, _)| m.eq_ignore_ascii_case(name));
                if row.is_some() {
                    state.vote_row = row;
                }
                if row.is_none() {
                    runlog::kv("map_vote_submit", &format!("map={name} via=pick refused=unknown_map"));
                } else {
                    submit(row, "pick");
                }
            } else if let Some(n) = r.strip_prefix("scroll:").and_then(|n| n.parse::<i64>().ok()).filter(|_| on) {
                state.vote_top = (state.vote_top as i64 + n).clamp(0, maps.len().saturating_sub(1) as i64) as usize;
            } else {
                runlog::kv("menu_action", &format!("action=vote.{r} refused=window_not_open"));
            }
        }
    }
    true
}

/// Column headings (XVoting.int MapVoteMultiColumnList ColumnHeadings;
/// MapVoteCountMultiColumnList's without its GameType column, which
/// ROMapVoteCountMultiColumnList drops) and their widths (a guess).
const MAP_COLUMNS: [(&str, f32); 3] = [("Map Name", 0.6), ("Played", 0.2), ("Seq", 0.2)];
const COUNT_COLUMNS: [(&str, f32); 2] = [("MapName", 0.75), ("Votes", 0.25)];

pub(super) fn draw(p: &mut Painter, state: &MenuState, d: &VoteDraw) {
    let gui = p.gui;
    let screen = p.screen;
    let (sw, sh) = (screen.width(), screen.height());
    let white = [255, 255, 255, 255];
    let win = Rect::new(WINDOW[0] * sw, WINDOW[1] * sh, (WINDOW[0] + WINDOW[2]) * sw, (WINDOW[1] + WINDOW[3]) * sh);
    // FloatingWindow: frame and title bar, as the pause menu.
    let title_font = named_font("UT2DefaultFont", sw);
    let th = p.line_height(title_font) + 8.0;
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Thin_border_SlightTransparent"), Rect::new(win.min.x, win.min.y + th * 0.5, win.max.x, win.max.y), white, "MapVote.FrameBG");
    let bar = Rect::new(win.min.x, win.min.y, win.max.x, win.min.y + th);
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Tabdark"), bar, white, "MapVote.TitleBar");
    p.text_in(title_font, TITLE, bar, Align::Center, true, [225, 225, 225, 255], "MapVote.Title");

    let comp = |path: &str, fallback: [f32; 4]| {
        let c = gui.comp(path);
        if c.win[2] > 0.0 && c.win[3] > 0.0 { c } else { super::gui::Comp { win: fallback, ..c } }
    };
    // KF_ServerBrowserGrid (STY2ServerBrowserGrid): UT2ServerListFont,
    // white text. Row height: the font's line plus 25% (a guess).
    let font = named_font("UT2ServerListFont", sw);
    let row_h = (p.line_height(font) * 1.25).round().max(1.0);
    let my_vote = d.view.vote_of(d.me);

    // The vote count list (ROMapVotingPage.VoteCountListBox) on its
    // background (MapCountListBackground, AlignBK: the list's box).
    let counts_box = comp("ROMapVotingPage.VoteCountListBox", [0.02, 0.077369, 0.96, 0.26752]).rect(win);
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Thin_border_SlightTransparent"), counts_box, white, "MapVote.CountBG");
    let ranked = d.view.ranked();
    let rows: Vec<(usize, Vec<String>)> = ranked.iter().map(|m| (d.view.maps.iter().position(|x| x.name == m.name).unwrap_or(0), vec![m.name.clone(), m.votes.to_string()])).collect();
    list(p, counts_box, &COUNT_COLUMNS, &rows, 0, state.vote_row, my_vote, d.view, row_h, font, "MapVote.Counts");

    // The map list (MapVotingPage.MapListBox; ROMapVotingPage has no
    // background image for it: a dark fill so the rows can be read, ours).
    let maps_box = comp("MapVotingPage.MapListBox", [0.02, 0.37102, 0.96, 0.293104]).rect(win);
    p.fill(maps_box, [0, 0, 0, 120], "MapVote.MapListBG");
    let rows: Vec<(usize, Vec<String>)> = d.view.maps.iter().enumerate().map(|(i, m)| (i, vec![m.name.clone(), m.plays.to_string(), m.seq.to_string()])).collect();
    let visible = ((maps_box.height() / row_h).floor() as usize).saturating_sub(1).max(1);
    let top = state.vote_top.min(rows.len().saturating_sub(visible));
    list(p, maps_box, &MAP_COLUMNS, &rows, top, state.vote_row, my_vote, d.view, row_h, font, "MapVote.Maps");
    // lmsgTotalMaps at the right of the list's heading (where KF shows
    // it is not known: ours).
    let total = format!("{} Total Maps", d.view.maps.len());
    let head = Rect::new(maps_box.min.x, maps_box.min.y + 6.0, maps_box.max.x - 10.0, maps_box.min.y + 6.0 + row_h);
    p.text_in(font, &total, head, Align::Right, true, [200, 200, 200, 200], "MapVote.TotalMaps");

    // The footer (VotingPage.MatchSetupFooter, MapVoteFooter).
    let footer = comp("VotingPage.MatchSetupFooter", [0.019921, 0.686457, 0.962109, 0.291406]).rect(win);
    let bg = comp("MapVoteFooter.MapvoteFooterBackground", [0.0, 0.0, 1.0, 0.82]).rect(footer);
    p.alt_section(bg, "Chat", "MapVote.FooterBG");
    let text_box = comp("MapVoteFooter.ChatScrollBox", [0.043845, 0.22358, 0.91897, 0.582534]).rect(bg);
    let mut lines: Vec<(String, [u8; 4])> = Vec::new();
    let name_of = |peer: u64| d.names.iter().find(|(p, _)| *p == peer).map_or_else(|| format!("Player {peer}"), |(_, n)| n.clone());
    // lmsgMapVotedFor for each vote (KF broadcasts one per vote).
    for (peer, map) in &d.view.voters {
        lines.push((format!("{} has voted for {map}(KF)", name_of(*peer)), [200, 200, 200, 255]));
    }
    match &d.view.result {
        Some(r) => lines.push((won_text(&r.map), [255, 255, 0, 255])),
        None => {
            if d.view.time_left >= 0 {
                lines.push((format!("Voting ends in {} seconds ({} of {} players voted)", d.view.time_left, d.view.voters.len(), d.view.players), white));
            }
            lines.push((my_vote.map_or("You have not voted yet.".to_string(), |m| format!("Your vote: {m}")), white));
        }
    }
    let font = named_font("UT2SmallFont", sw);
    let lh = p.line_height(font);
    let fit = ((text_box.height() / lh).floor() as usize).max(1);
    let skip = lines.len().saturating_sub(fit);
    for (i, (line, color)) in lines.iter().skip(skip).enumerate() {
        p.text(font, line, Vec2::new(text_box.min.x, text_box.min.y + i as f32 * lh), *color, "MapVote.Status");
    }
    let submit = gui.comp("MapVoteFooter.SubmitButton");
    let submit_box = comp("MapVoteFooter.SubmitButton", [0.704931, 0.849625, 0.160075, 0.165403]).rect(footer);
    let can_submit = d.view.result.is_none() && state.vote_row.and_then(|r| d.view.maps.get(r)).is_some_and(|m| m.enabled);
    p.button("vote.submit", submit_box, if submit.caption.is_empty() { "Submit" } else { &submit.caption }, if can_submit { State::Blurry } else { State::Disabled });
    let close = gui.comp("MapVoteFooter.CloseButton");
    let close_box = comp("MapVoteFooter.CloseButton", [0.861895, 0.849625, 0.137744, 0.165403]).rect(footer);
    p.button("vote.close", close_box, if close.caption.is_empty() { "Close" } else { &close.caption }, State::Blurry);
}

/// A multi-column list: the heading row, then the rows from `top` that
/// fit. Each row is clickable (`vote.row:<map index>`). The selected
/// row is highlighted (BrowserListSelection: a guess), my vote marked,
/// disabled maps grey.
#[allow(clippy::too_many_arguments)]
fn list(p: &mut Painter, rect: Rect, columns: &[(&str, f32)], rows: &[(usize, Vec<String>)], top: usize, selected: Option<usize>, my_vote: Option<&str>, view: &VoteView, row_h: f32, font: &str, what: &str) {
    let pad = 6.0;
    let inner = Rect::new(rect.min.x + pad, rect.min.y + pad, rect.max.x - pad, rect.max.y - pad);
    // Column x positions.
    let mut xs = Vec::new();
    let mut x = inner.min.x;
    for (_, w) in columns {
        xs.push(x);
        x += w * inner.width();
    }
    let head = Rect::new(inner.min.x, inner.min.y, inner.max.x, inner.min.y + row_h);
    p.fill(head, [0, 0, 0, 160], &format!("{what}.Header"));
    for (i, (h, _)) in columns.iter().enumerate() {
        p.text_in(font, h, Rect::new(xs[i] + 4.0, head.min.y, inner.max.x, head.max.y), Align::Left, true, [225, 225, 225, 255], &format!("{what}.Heading"));
    }
    let mut y = head.max.y;
    for (map, cells) in rows.iter().skip(top) {
        if y + row_h > inner.max.y + 0.5 {
            break;
        }
        let r = Rect::new(inner.min.x, y, inner.max.x, y + row_h);
        let m = &view.maps[*map];
        if selected == Some(*map) {
            p.fill(r, [120, 20, 20, 200], &format!("{what}.Selected"));
        } else if p.hover(r) {
            p.fill(r, [255, 255, 255, 40], &format!("{what}.Hover"));
        }
        let mine = my_vote.is_some_and(|v| v.eq_ignore_ascii_case(&m.name));
        let color = if !m.enabled {
            [110, 110, 110, 255]
        } else if mine {
            [255, 255, 0, 255]
        } else {
            [225, 225, 225, 255]
        };
        for (i, c) in cells.iter().enumerate() {
            let text = if i == 0 && mine { format!("{c}  <") } else { c.clone() };
            p.text_in(font, &text, Rect::new(xs[i] + 4.0, r.min.y, inner.max.x, r.max.y), Align::Left, true, color, &format!("{what}.Cell"));
        }
        p.hit(&format!("vote.row:{map}"), r);
        y += row_h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actions_map_to_window_ids() {
        assert_eq!(scripted_ids("vote:KF-Farm").as_deref(), Some("vote.pick:KF-Farm"));
        assert_eq!(scripted_ids("vote_row:3").as_deref(), Some("vote.row:3"));
        assert_eq!(scripted_ids("vote_submit").as_deref(), Some("vote.submit"));
        assert_eq!(scripted_ids("vote_scroll:5").as_deref(), Some("vote.scroll:5"));
        assert_eq!(scripted_ids("lobby_ready"), None);
    }
}
