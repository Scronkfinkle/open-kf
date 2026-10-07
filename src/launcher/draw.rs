//! The launcher's screen, drawn with the KF menu pieces in
//! `game/menus/gui.rs` (section boxes, buttons, KF's fonts). The layout
//! is our own (KF has no such page); sizes are fractions of the window so
//! it fits any size.

use bevy::prelude::*;

use super::choices::{LENGTHS, PlayType, command_line};
use super::{Field, Launcher};
use crate::game::menus::gui::{Align, FONTS, Painter, State, named_font};

const WHITE: [u8; 4] = [255, 255, 255, 255];
const LABEL: [u8; 4] = [200, 200, 200, 255];
const DIM: [u8; 4] = [140, 140, 140, 255];
const NOTE: [u8; 4] = [255, 200, 80, 255];
const FIELD_BG: [u8; 4] = [0, 0, 0, 170];
const FIELD_FOCUS: [u8; 4] = [60, 20, 20, 220];

/// Rows inside a section: a label on the left, the control on the right.
struct Rows {
    area: Rect,
    y: f32,
    h: f32,
    gap: f32,
    label_w: f32,
}

impl Rows {
    fn new(area: Rect, h: f32, gap: f32, label_frac: f32) -> Self {
        Rows { area, y: area.min.y, h, gap, label_w: area.width() * label_frac }
    }

    /// The next row: (label box, control box).
    fn next(&mut self) -> (Rect, Rect) {
        let r = Rect::new(self.area.min.x, self.y, self.area.max.x, self.y + self.h);
        self.y += self.h + self.gap;
        (Rect::new(r.min.x, r.min.y, r.min.x + self.label_w, r.max.y), Rect::new(r.min.x + self.label_w, r.min.y, r.max.x, r.max.y))
    }
}

/// The height a section needs for `rows` rows (its header and borders:
/// ImageOffset 35 + 10 px, see `Painter::section_client`).
fn section_height(rows: usize, h: f32, gap: f32) -> f32 {
    45.0 + rows as f32 * (h + gap)
}

pub fn draw(p: &mut Painter, l: &mut Launcher) {
    let gui = p.gui;
    let screen = p.screen;
    let (sw, sh) = (screen.width(), screen.height());
    let menu = named_font("UT2MenuFont", sw);
    let small = named_font("UT2SmallFont", sw);
    let rh = (p.line_height(menu) * 1.6).round();
    let gap = (rh * 0.3).round();

    // The window: KF's floating frame and title bar (as the pause menu).
    let win = Rect::new(0.02 * sw, 0.02 * sh, 0.98 * sw, 0.98 * sh);
    let title_font = named_font("UT2DefaultFont", sw);
    let bar = Rect::new(win.min.x, win.min.y, win.max.x, win.min.y + p.line_height(title_font) + 8.0);
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Thin_border_SlightTransparent"), Rect::new(win.min.x, bar.max.y, win.max.x, win.max.y), WHITE, "Frame");
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Tabdark"), bar, WHITE, "TitleBar");
    p.text_in(title_font, "Open KF", bar, Align::Center, true, [225, 225, 225, 255], "Title");

    // Bottom area: extra arguments, the command, status, QUIT / PLAY.
    let inner = Rect::new(win.min.x + 0.015 * sw, bar.max.y + gap, win.max.x - 0.015 * sw, win.max.y - gap);
    let bottom_h = rh * 3.0 + gap * 3.0;
    let content = Rect::new(inner.min.x, inner.min.y, inner.max.x, inner.max.y - bottom_h);
    let col_w = (content.width() - 2.0 * gap) / 3.0;
    let col = |i: f32| Rect::new(content.min.x + i * (col_w + gap), content.min.y, content.min.x + i * (col_w + gap) + col_w, content.max.y);
    let (a, b, c) = (col(0.0), col(1.0), col(2.0));
    let c_ = &l.choices.clone();
    // (A copy: the text fields below borrow `l` mutably.)
    let join = c_.play == PlayType::Join;

    // Column 1: Play, then Player.
    // Solo: the buttons and a note; Host / Join: a field too.
    let play_rows = match c_.play {
        PlayType::Solo => 2,
        PlayType::Host => 3,
        PlayType::Join => 4,
    };
    let play_h = section_height(play_rows, rh, gap);
    let play = Rect::new(a.min.x, a.min.y, a.max.x, a.min.y + play_h);
    p.section(play, "Play", false, "Play");
    let mut rows = Rows::new(Painter::section_client(play, [0.0; 4]), rh, gap, 0.35);
    let (lab, ctl) = rows.next();
    let full = Rect::new(lab.min.x, lab.min.y, ctl.max.x, ctl.max.y);
    let w = (full.width() - 2.0 * gap) / 3.0;
    for (i, (t, cap)) in [(PlayType::Solo, "Solo"), (PlayType::Host, "Host"), (PlayType::Join, "Join")].into_iter().enumerate() {
        let x = full.min.x + i as f32 * (w + gap);
        let state = if c_.play == t { State::Focused } else { State::Blurry };
        p.button(&format!("play:{}", t.word()), Rect::new(x, full.min.y, x + w, full.max.y), cap, state);
    }
    match c_.play {
        PlayType::Solo => {
            let (lab, ctl) = rows.next();
            let full = Rect::new(lab.min.x, lab.min.y, ctl.max.x, ctl.max.y);
            note(p, full, "Play alone on this computer.", small, DIM, "PlayNote");
        }
        PlayType::Host => {
            let (lab, ctl) = rows.next();
            label(p, lab, "Port", menu);
            text_field(p, l, Field::Port, ctl, menu);
            let (lab, ctl) = rows.next();
            let full = Rect::new(lab.min.x, lab.min.y, ctl.max.x, ctl.max.y);
            let ports = match c_.port.trim().parse::<u16>() {
                Ok(n) if n < u16::MAX => format!("UDP ports {n}-{} open", n + 1),
                _ => "UDP ports P and P+1 open".into(),
            };
            note(p, full, &format!("Others join your address ({ports})."), small, DIM, "HostNote");
        }
        PlayType::Join => {
            let (lab, ctl) = rows.next();
            label(p, lab, "Address", menu);
            text_field(p, l, Field::Address, ctl, menu);
            let (lab, ctl) = rows.next();
            let full = Rect::new(lab.min.x, lab.min.y, ctl.max.x, ctl.max.y);
            note(p, full, "The map, mode and length come from the host.", small, DIM, "JoinNote");
            // CHECK HOST and its answer.
            let (lab, ctl) = rows.next();
            let bw = (p.text_size(menu, "Check host").x + rh).min(lab.width() + ctl.width() * 0.5);
            let waiting = l.host_wait.is_some();
            p.button("check_host", Rect::new(lab.min.x, lab.min.y, lab.min.x + bw, lab.max.y), "Check host", if waiting { State::Disabled } else { State::Blurry });
            let hint = if waiting { "Asking the host..." } else { "Asks the host what it plays." };
            note(p, Rect::new(lab.min.x + bw + gap, lab.min.y, ctl.max.x, ctl.max.y), hint, small, DIM, "HostHint");
        }
    }

    let player = Rect::new(a.min.x, play.max.y + gap, a.max.x, a.max.y);
    p.section(player, "Player", false, "Player");
    let client = Painter::section_client(player, [0.0; 4]);
    let mut rows = Rows::new(client, rh, gap, 0.35);
    let (lab, ctl) = rows.next();
    label(p, lab, "Name", menu);
    text_field(p, l, Field::Name, ctl, menu);
    let (lab, ctl) = rows.next();
    label(p, lab, "Perk", menu);
    let perk = c_.perk.map_or("None".to_string(), |k| k.name().to_string());
    spinner(p, "perk", ctl, &perk, true, menu);
    let (lab, ctl) = rows.next();
    label(p, lab, "Perk level", menu);
    spinner(p, "level", ctl, &c_.perk_level.to_string(), c_.perk.is_some(), menu);
    let (lab, ctl) = rows.next();
    label(p, lab, "Character", menu);
    spinner(p, "character", ctl, &c_.character.replace('_', " "), !l.characters.is_empty(), menu);
    // The perk icon and the character's portrait below.
    let pic = Rect::new(client.min.x, rows.y, client.max.x, client.max.y);
    if pic.height() > 20.0 {
        if let Some(k) = c_.perk {
            let s = pic.height().min(pic.width() * 0.35).min(rh * 3.0);
            p.tile(gui.tex(k.icons().0), Rect::new(pic.min.x, pic.min.y, pic.min.x + s, pic.min.y + s), WHITE, "PerkIcon");
        }
        let portrait = l.characters.iter().find(|(n, _)| n.eq_ignore_ascii_case(&c_.character)).and_then(|(_, t)| gui.tex(t));
        if let Some(t) = portrait {
            let size = gui.textures[t].size;
            let h = pic.height();
            let w = (h * size.x / size.y.max(1.0)).min(pic.width() * 0.6);
            let h = w * size.y / size.x.max(1.0);
            let x = pic.max.x - w;
            p.tile(Some(t), Rect::new(x, pic.min.y, x + w, pic.min.y + h), WHITE, "Portrait");
        }
    }

    // Column 2: the map list.
    p.section(b, "Map", false, "Map");
    let list = Painter::section_client(b, [0.0; 4]);
    if join {
        p.scroll_text(menu, "Joining: the host's map is used.", list, DIM, "MapJoin");
        // The CHECK HOST answer (map - mode length - players).
        if let Some((_, t)) = &l.host_text {
            let y = list.min.y + p.line_height(menu) * 2.0;
            let t = t.replace(" - ", "|");
            p.scroll_text(menu, &format!("Host:|{t}"), Rect::new(list.min.x, y, list.max.x, list.max.y), WHITE, "HostAnswer");
        }
    } else {
        map_list(p, l, list, menu);
    }

    // Column 3: Game, then Display, sound, menus (3 + 5 rows). On a short
    // window its rows get a little smaller so both fit above the bottom row.
    let need = |h: f32, g: f32| section_height(3, h, g) + g + section_height(5, h, g);
    let (rh3, gap3) = if need(rh, gap) <= c.height() {
        (rh, gap)
    } else {
        let s = ((c.height() - 90.0 - gap) / (8.0 * (rh + gap))).max(0.5);
        ((rh * s).floor(), (gap * s).floor())
    };
    let game_h = section_height(3, rh3, gap3);
    let game = Rect::new(c.min.x, c.min.y, c.max.x, c.min.y + game_h);
    p.section(game, "Game", false, "Game");
    let mut rows = Rows::new(Painter::section_client(game, [0.0; 4]), rh3, gap3, 0.4);
    if join {
        let (lab, ctl) = rows.next();
        note(p, Rect::new(lab.min.x, lab.min.y, ctl.max.x, ctl.max.y), "Set by the host.", small, DIM, "GameJoin");
    } else {
        let (lab, ctl) = rows.next();
        label(p, lab, "Mode", menu);
        spinner(p, "mode", ctl, if c_.waves { "Waves" } else { "Debug (no waves)" }, true, menu);
        let (lab, ctl) = rows.next();
        label(p, lab, "Length", menu);
        let len = LENGTHS[c_.length.min(2)];
        spinner(p, "length", ctl, &format!("{}{}", len[..1].to_uppercase(), &len[1..]), c_.waves, menu);
        let (lab, ctl) = rows.next();
        label(p, lab, "Start at wave", menu);
        spinner(p, "wave", ctl, &c_.start_wave.map_or("First".into(), |w| w.to_string()), c_.waves, menu);
    }

    let disp = Rect::new(c.min.x, game.max.y + gap3, c.max.x, game.max.y + gap3 + section_height(5, rh3, gap3));
    p.section(disp, "Display, sound, menus", false, "Display");
    let mut rows = Rows::new(Painter::section_client(disp, [0.0; 4]), rh3, gap3, 0.4);
    let (lab, ctl) = rows.next();
    label(p, lab, "Window", menu);
    spinner(p, "window", ctl, &c_.window.map_or("Default".into(), |(w, h)| format!("{w} x {h}")), true, menu);
    let (lab, ctl) = rows.next();
    label(p, lab, "Frame limit", menu);
    spinner(p, "fps", ctl, &c_.fps.map_or("None".into(), |f| format!("{f} fps")), true, menu);
    let (lab, ctl) = rows.next();
    label(p, lab, "Vsync", menu);
    spinner(p, "vsync", ctl, if c_.vsync { "On" } else { "Off" }, true, menu);
    let (lab, ctl) = rows.next();
    label(p, lab, "Sound", menu);
    spinner(p, "sound", ctl, if c_.sound { "On" } else { "Off (muted)" }, true, menu);
    let (lab, ctl) = rows.next();
    label(p, lab, "Trader menu", menu);
    spinner(p, "trader", ctl, if c_.trader == crate::game::buy_menu::MenuKind::Nu { "NuMenu (new)" } else { "KF classic" }, true, menu);


    // The bottom rows.
    let mut y = content.max.y + gap;
    let bw = (p.text_size(menu, "PLAY").x * 3.0).max(rh * 3.0);
    let extra_lab = Rect::new(inner.min.x, y, inner.min.x + col_w * 0.45, y + rh);
    label(p, extra_lab, "Extra arguments", menu);
    text_field(p, l, Field::Extra, Rect::new(extra_lab.max.x, y, inner.max.x - 2.0 * bw - 2.0 * gap, y + rh), menu);
    let quit = Rect::new(inner.max.x - 2.0 * bw - gap, y, inner.max.x - bw - gap, y + rh);
    p.button("quit", quit, "QUIT", State::Blurry);
    let problem = c_.problem();
    let play_b = Rect::new(inner.max.x - bw, y, inner.max.x, y + rh);
    p.button("launch", play_b, "PLAY", if problem.is_some() { State::Disabled } else { State::Blurry });
    if problem.is_some() {
        p.fill(play_b, [0, 0, 0, 140], "PlayDisabled");
    }
    y += rh + gap;
    let cmd = match c_.to_args(false) {
        Ok(a) => format!("open-kf {}", command_line(&a)),
        Err(_) => String::new(),
    };
    let line_box = Rect::new(inner.min.x, y, inner.max.x, inner.max.y);
    let message = if !l.status.is_empty() { Some(l.status.clone()) } else { problem };
    if let Some(m) = message {
        note(p, Rect::new(line_box.min.x, y, line_box.max.x, y + rh), &m, menu, NOTE, "Status");
        y += rh;
    }
    if !cmd.is_empty() {
        let lines = p.wrap(small, &format!("Command: {cmd}"), line_box.width()).len();
        p.scroll_text(small, &format!("Command: {cmd}"), Rect::new(line_box.min.x, y, line_box.max.x, line_box.max.y), DIM, "Command");
        y += lines as f32 * p.line_height(small);
    }
    // A short help text at the bottom.
    let help = "Click a field to type in it (Tab: next field). Enter: play. Escape: quit. Your choices are saved when you press PLAY.";
    let hh = p.line_height(small);
    note(p, Rect::new(line_box.min.x, y + 4.0, line_box.max.x, y + 4.0 + hh), help, small, DIM, "Help");
}

/// The largest of KF's menu fonts, up to `font`, that fits `text` into
/// `width` (the smallest if none does).
fn fit(p: &Painter, font: &'static str, text: &str, width: f32) -> &'static str {
    let start = FONTS.iter().position(|f| *f == font).unwrap_or(FONTS.len() - 1);
    FONTS[..=start].iter().rev().copied().find(|f| p.text_size(f, text).x <= width).unwrap_or(FONTS[0])
}

/// A one-line note in a box, in a font that fits.
fn note(p: &mut Painter, r: Rect, text: &str, font: &'static str, color: [u8; 4], what: &str) {
    let f = fit(p, font, text, r.width());
    p.text_in(f, text, r, Align::Left, true, color, what);
}

fn label(p: &mut Painter, r: Rect, text: &str, font: &str) {
    p.text_in(font, text, Rect::new(r.min.x, r.min.y, r.max.x - 6.0, r.max.y), Align::Left, true, LABEL, &format!("Label.{text}"));
}

/// `<` value `>`: the arrows step the choice (`spin:FIELD:-1` / `:1`).
fn spinner(p: &mut Painter, field: &str, r: Rect, value: &str, enabled: bool, font: &'static str) {
    let w = r.height();
    let st = if enabled { State::Blurry } else { State::Disabled };
    p.button(&format!("spin:{field}:-1"), Rect::new(r.min.x, r.min.y, r.min.x + w, r.max.y), "<", st);
    p.button(&format!("spin:{field}:1"), Rect::new(r.max.x - w, r.min.y, r.max.x, r.max.y), ">", st);
    let mid = Rect::new(r.min.x + w + 2.0, r.min.y, r.max.x - w - 2.0, r.max.y);
    p.fill(mid, FIELD_BG, &format!("Spin.{field}"));
    let font = fit(p, font, value, mid.width() - 8.0);
    p.text_in(font, value, mid, Align::Center, true, if enabled { WHITE } else { DIM }, &format!("Spin.{field}.Value"));
}

/// A box to type in; a click focuses it (`focus:FIELD`).
fn text_field(p: &mut Painter, l: &mut Launcher, f: Field, r: Rect, font: &'static str) {
    let focused = l.focus == Some(f);
    p.fill(r, if focused { FIELD_FOCUS } else { FIELD_BG }, &format!("Field.{}", f.key()));
    p.stretched(p.gui.tex("KF_InterfaceArt_tex.Menu.Thin_border"), r, if focused { WHITE } else { [255, 255, 255, 120] }, &format!("Field.{}.Border", f.key()));
    let mut text = f.text(&mut l.choices).clone();
    if focused {
        text.push('_');
    } else if text.is_empty() && f == Field::Name {
        // The game then uses KF's default name (defuser.ini).
        p.text_in(fit(p, font, "(KF's default)", r.width() - 16.0), "(KF's default)", Rect::new(r.min.x + 8.0, r.min.y, r.max.x - 8.0, r.max.y), Align::Left, true, DIM, "Field.name.Hint");
    }
    // Show the end of a long text.
    let room = r.width() - 16.0;
    while p.text_size(font, &text).x > room && !text.is_empty() {
        text.remove(0);
    }
    p.text_in(font, &text, Rect::new(r.min.x + 8.0, r.min.y, r.max.x - 8.0, r.max.y), Align::Left, true, WHITE, &format!("Field.{}.Text", f.key()));
    p.hit(&format!("focus:{}", f.key()), r);
}

/// The map list: one row per map, the chosen one highlighted (`map:I`).
fn map_list(p: &mut Painter, l: &mut Launcher, r: Rect, font: &str) {
    p.fill(r, FIELD_BG, "MapList");
    let row_h = (p.line_height(font) * 1.35).round();
    let rows = ((r.height() / row_h).floor() as usize).max(1);
    l.map_rows = rows;
    let n = l.maps.len();
    let chosen = l.maps.iter().position(|m| m.eq_ignore_ascii_case(&l.choices.map));
    if l.reveal_map {
        l.reveal_map = false;
        if let Some(i) = chosen
            && (i < l.map_top || i >= l.map_top + rows)
        {
            l.map_top = i.saturating_sub(rows / 2);
        }
    }
    l.map_top = l.map_top.min(n.saturating_sub(rows));
    let bar_w = 10.0;
    for (k, i) in (l.map_top..n.min(l.map_top + rows)).enumerate() {
        let y = r.min.y + k as f32 * row_h;
        let row = Rect::new(r.min.x, y, r.max.x - bar_w - 2.0, y + row_h);
        if Some(i) == chosen {
            p.fill(row, [150, 20, 20, 230], "MapChosen");
        } else if p.hover(row) {
            p.fill(row, [255, 255, 255, 40], "MapHover");
        }
        p.text_in(font, &l.maps[i], Rect::new(row.min.x + 8.0, row.min.y, row.max.x, row.max.y), Align::Left, true, WHITE, "MapName");
        p.hit(&format!("map:{i}"), row);
    }
    // Where the shown rows are in the whole list.
    if n > rows {
        let track = Rect::new(r.max.x - bar_w, r.min.y, r.max.x, r.max.y);
        p.fill(track, [255, 255, 255, 30], "MapTrack");
        let h = track.height() * rows as f32 / n as f32;
        let top = track.min.y + track.height() * l.map_top as f32 / n as f32;
        p.fill(Rect::new(track.min.x, top, track.max.x, top + h), [200, 200, 200, 160], "MapThumb");
    }
    if n == 0 {
        p.scroll_text(font, "No maps found in the install's Maps folder.", r, NOTE, "MapNone");
    }
}
