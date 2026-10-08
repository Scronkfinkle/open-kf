//! The Audio window opened by the pause menu's Settings button. KF opens
//! its whole Settings page there (UT2K4SettingsPage, with an Audio tab:
//! KFGui.KFAudioSettingsTab); ours is one small window with that tab's
//! "Sound System" box (AudioBK1) and its two volume sliders
//! (AudioEffectsVolumeSlider, AudioMusicVolume), plus our master volume,
//! and a "Controls" box with the aim mode (Toggle / Hold; KF sets that
//! by binding the key to ToggleAiming or Aiming), the mouse sensitivity
//! and invert mouse (KF's Input tab: KFGui.KFInputSettings). See
//! DESIGN.md, "Volume control", "Aim down sights" and "Mouse sensitivity
//! and invert mouse".

use bevy::prelude::*;

use super::gui::{Align, Painter, State, named_font};
use crate::launcher::choices::SLIDERS;

/// What the window shows: the volume settings, the ini's values and
/// whether `--mute` is on.
pub(crate) struct AudioView {
    pub volumes: crate::launcher::choices::Volumes,
    pub ini: (f32, f32),
    pub muted: bool,
}

/// The click id of an aim mode button (`aim.mode:toggle` / `aim.mode:hold`).
pub(crate) fn aim_id(hold: bool) -> String {
    format!("aim.mode:{}", crate::launcher::choices::aim_word(hold))
}

/// The keyboard row of the aim mode (after the sliders).
pub(crate) const AIM_ROW: usize = SLIDERS.len();
/// The keyboard rows of the mouse sensitivity and invert mouse.
pub(crate) const SENS_ROW: usize = AIM_ROW + 1;
pub(crate) const INVERT_ROW: usize = AIM_ROW + 2;
/// All the rows Up / Down go through.
pub(crate) const ROWS: usize = AIM_ROW + 3;
/// The click id of the mouse sensitivity slider.
pub(crate) const SENS_ID: &str = "mouse.sensitivity";

/// The click id of an invert mouse button (`mouse.invert:off` / `:on`).
pub(crate) fn invert_id(on: bool) -> String {
    format!("mouse.invert:{}", crate::launcher::choices::on_off_word(on))
}

/// What the Controls box shows: the aim mode, the mouse settings and
/// whether the sensitivity slider is being dragged.
pub(crate) struct ControlsView {
    pub aim_hold: bool,
    pub sensitivity: f32,
    pub invert: bool,
    pub sens_drag: bool,
}

/// The click id of a slider (`volume.slider:master` ...).
pub(crate) fn slider_id(name: &str) -> String {
    format!("volume.slider:{name}")
}

/// A slider's caption: KF's (from KFGui.u) for its two, ours for master.
fn caption(p: &Painter, name: &str, ours: &str) -> String {
    let path = match name {
        "effects" => "KFAudioSettingsTab.AudioEffectsVolumeSlider",
        "music" => "KFAudioSettingsTab.AudioMusicVolume",
        _ => return ours.to_string(),
    };
    let c = p.gui.comp(path).caption;
    if c.is_empty() { ours.to_string() } else { c }
}

/// `row`: the slider (or a Controls row) picked with the Up / Down keys;
/// `drag`: the volume slider the mouse holds; `ctl`: the Controls box.
pub(crate) fn draw(p: &mut Painter, view: Option<&AudioView>, row: usize, drag: Option<usize>, ctl_view: &ControlsView) {
    let gui = p.gui;
    let white = [255, 255, 255, 255];
    let (sw, sh) = (p.screen.width(), p.screen.height());
    let menu = named_font("UT2MenuFont", sw);
    let small = named_font("UT2SmallFont", sw);
    let title_font = named_font("UT2DefaultFont", sw);
    let rh = (p.line_height(menu) * 1.6).round();
    let gap = (rh * 0.4).round();
    let th = p.line_height(title_font) + 8.0;
    // Our layout (KF's page is full screen): a window in the middle, the
    // title bar, the sections (header 35 + rows + 10 px, see
    // `Painter::section_client`), a note line and the Back button.
    let rows = SLIDERS.len() as f32;
    let section_h = 45.0 + rows * rh + (rows - 1.0) * gap;
    // Controls: aim mode, mouse sensitivity, invert mouse.
    let controls_h = 45.0 + 3.0 * rh + 2.0 * gap;
    let h = th + gap + section_h + gap + p.line_height(small) + gap + controls_h + gap + rh + gap;
    let w = (0.5 * sw).max(420.0).min(sw);
    let win = Rect::new((sw - w) / 2.0, (sh - h) / 2.0, (sw + w) / 2.0, (sh + h) / 2.0);
    // A dark backing (ours): the frame texture is see-through and the
    // pause menu's text showed through it.
    p.fill(win, [10, 8, 8, 235], "Audio.Backing");
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Thin_border_SlightTransparent"), win, white, "Audio.Frame");
    let bar = Rect::new(win.min.x, win.min.y, win.max.x, win.min.y + th);
    p.stretched(gui.tex("KF_InterfaceArt_tex.Menu.Tabdark"), bar, white, "Audio.TitleBar");
    // UT2K4SettingsPage's caption (KF's page holds the Audio tab and
    // the controls).
    p.text_in(title_font, "Settings", bar, Align::Center, true, [225, 225, 225, 255], "Audio.Title");

    let pad = 0.03 * w;
    let sec = Rect::new(win.min.x + pad, bar.max.y + gap, win.max.x - pad, bar.max.y + gap + section_h);
    let bk = gui.comp("KFAudioSettingsTab.AudioBK1").caption;
    p.section(sec, if bk.is_empty() { "Sound System" } else { &bk }, false, "Audio.BK1");
    let client = Painter::section_client(sec, [0.0; 4]);
    // The Controls box (ours, see the module doc), under the note line.
    let note_y = sec.max.y + gap;
    let ctl_top = note_y + p.line_height(small) + gap;
    let ctl = Rect::new(sec.min.x, ctl_top, sec.max.x, ctl_top + controls_h);
    p.section(ctl, "Controls", false, "Audio.Controls");
    let crow = Painter::section_client(ctl, [0.0; 4]);
    let crow = Rect::new(crow.min.x, crow.min.y, crow.max.x, crow.min.y + rh);
    let cw = crow.width() * 0.4;
    let lit = row == AIM_ROW;
    p.text_in(menu, "Aim down sights", Rect::new(crow.min.x, crow.min.y, crow.min.x + cw, crow.max.y), Align::Left, true, if lit { white } else { [200, 200, 200, 255] }, "Audio.Aim.Caption");
    let bgap = (rh * 0.3).round();
    let bw = (crow.width() - cw - bgap) / 2.0;
    for (i, (hold, cap)) in [(false, "Toggle"), (true, "Hold")].into_iter().enumerate() {
        let x = crow.min.x + cw + i as f32 * (bw + bgap);
        // The chosen one lit (as the launcher's Solo / Host / Join).
        let state = if hold == ctl_view.aim_hold { State::Focused } else { State::Blurry };
        p.button(&aim_id(hold), Rect::new(x, crow.min.y, x + bw, crow.max.y), cap, state);
    }
    // Mouse Sensitivity (Game): KF's box (0.25 to 25, steps of 0.25) as a
    // slider with the number at the right, as the volume rows.
    let srow = Rect::new(crow.min.x, crow.max.y + gap, crow.max.x, crow.max.y + gap + rh);
    let lit = row == SENS_ROW || ctl_view.sens_drag;
    p.text_in(menu, "Mouse Sensitivity", Rect::new(srow.min.x, srow.min.y, srow.min.x + cw, srow.max.y), Align::Left, true, if lit { white } else { [200, 200, 200, 255] }, "Audio.Sensitivity.Caption");
    let vw = srow.width() * 0.14;
    let sl = Rect::new(srow.min.x + cw, srow.min.y + rh * 0.15, srow.max.x - vw - 8.0, srow.max.y - rh * 0.15);
    p.slider(SENS_ID, sl, crate::launcher::choices::sensitivity_fraction(ctl_view.sensitivity), lit);
    p.text_in(menu, &format!("{:.2}", ctl_view.sensitivity), Rect::new(srow.max.x - vw, srow.min.y, srow.max.x, srow.max.y), Align::Right, true, white, "Audio.Sensitivity.Value");
    // Invert Mouse: KF's checkbox, as Off / On buttons (the aim row's look).
    let irow = Rect::new(crow.min.x, srow.max.y + gap, crow.max.x, srow.max.y + gap + rh);
    let lit = row == INVERT_ROW;
    p.text_in(menu, "Invert Mouse", Rect::new(irow.min.x, irow.min.y, irow.min.x + cw, irow.max.y), Align::Left, true, if lit { white } else { [200, 200, 200, 255] }, "Audio.Invert.Caption");
    for (i, (on, cap)) in [(false, "Off"), (true, "On")].into_iter().enumerate() {
        let x = irow.min.x + cw + i as f32 * (bw + bgap);
        let state = if on == ctl_view.invert { State::Focused } else { State::Blurry };
        p.button(&invert_id(on), Rect::new(x, irow.min.y, x + bw, irow.max.y), cap, state);
    }
    let bw = p.text_size(menu, "Back").x + 0.04 * sw * 0.25 + rh;
    let by = win.max.y - gap - rh;
    p.button("audio.back", Rect::new(win.max.x - pad - bw, by, win.max.x - pad, by + rh), "Back", State::Blurry);
    let Some(view) = view else {
        p.text_in(menu, "No sound system.", client, Align::Left, false, white, "Audio.None");
        return;
    };
    for (i, (name, ours)) in SLIDERS.iter().enumerate() {
        let y = client.min.y + i as f32 * (rh + gap);
        let row_box = Rect::new(client.min.x, y, client.max.x, y + rh);
        let cw = row_box.width() * 0.4;
        let vw = row_box.width() * 0.14;
        let lit = row == i || drag == Some(i);
        let cap = caption(p, name, ours);
        // GUIMenuOption caption, then the slider; the number at the right
        // is ours (KF shows it in a tooltip while dragging).
        p.text_in(menu, &cap, Rect::new(row_box.min.x, y, row_box.min.x + cw, y + rh), Align::Left, true, if lit { white } else { [200, 200, 200, 255] }, &format!("Audio.{name}.Caption"));
        let (v, max) = view.volumes.slider(name, view.ini).unwrap_or((0.0, 1.0));
        let sl = Rect::new(row_box.min.x + cw, y + rh * 0.15, row_box.max.x - vw - 8.0, y + rh * 0.85);
        p.slider(&slider_id(name), sl, v / max, lit);
        p.text_in(menu, &format!("{v:.2}"), Rect::new(row_box.max.x - vw, y, row_box.max.x, y + rh), Align::Right, true, white, &format!("Audio.{name}.Value"));
    }
    let note = if view.muted { "--mute is on: the speakers stay silent whatever the volume." } else { "Effects and Music: KF's sliders (0 to 0.5). Master: both together." };
    p.text_in(small, note, Rect::new(sec.min.x, note_y, sec.max.x, note_y + p.line_height(small)), Align::Left, false, [200, 200, 200, 200], "Audio.Note");
}
