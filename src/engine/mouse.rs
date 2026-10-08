//! Mouse look settings, as KF's: the mouse sensitivity (KF's
//! MouseSensitivity, default 3) and invert mouse (bInvertMouse), and how
//! a raw mouse count becomes a turn of the view. Saved as
//! `mouse_sensitivity=` and `invert_mouse=` in the launcher's settings
//! file; `--sensitivity` / `--invert-mouse` override them for one run;
//! the pause menu's Settings window changes them at once. See DESIGN.md,
//! "Mouse sensitivity and invert mouse".

use bevy::prelude::*;

use crate::engine::runlog;
use crate::launcher::choices;

/// Unreal rotation units one raw mouse count turns the view at
/// sensitivity 1 and FOV 90 (65536 units = one full turn). KF: each
/// count adds Speed 2.0 x 0.01 to the mouse axis, the engine multiplies
/// the axes by 20 / frame time, and the view turns 32 x frame time x
/// axis; the frame time cancels: 2 x 0.01 x 20 x 32 = 12.8. Read from
/// KF (scripts and engine code; details in the local RE.md).
pub const UNITS_PER_COUNT: f32 = 2.0 * 0.01 * 20.0 * 32.0;

/// The FOV KF scales the mouse by while a 3D scope is drawn
/// (KFPlayerController.GetMouseModifier: 24 with KF_ModelScope).
pub const SCOPE_MOUSE_FOV: f32 = 24.0;

/// KFPlayerInput's FOVScale: the field of view (horizontal degrees at
/// 4:3, as KF's) / 90, or 24 / 90 while a 3D scope is drawn.
pub fn fov_scale(view_fov_deg: f32, scope_drawn: bool) -> f32 {
    (if scope_drawn { SCOPE_MOUSE_FOV } else { view_fov_deg }) / 90.0
}

/// Radians the view turns for one raw mouse count.
pub fn radians_per_count(sensitivity: f32, fov_scale: f32) -> f32 {
    UNITS_PER_COUNT * sensitivity * fov_scale * std::f32::consts::TAU / 65536.0
}

/// The turn for a mouse movement of `delta` raw counts (x right, y down,
/// as the window system reports them): (yaw change, pitch change) in
/// radians, in our camera's terms (yaw grows turning left, pitch grows
/// looking up). Not inverted: moving the mouse forward (y < 0) looks up,
/// as KF.
pub fn look_delta(delta: Vec2, sensitivity: f32, invert: bool, fov_scale: f32) -> (f32, f32) {
    let r = radians_per_count(sensitivity, fov_scale);
    let up = if invert { delta.y } else { -delta.y };
    (-delta.x * r, up * r)
}

/// The mouse settings in use and where they are saved.
#[derive(Resource, Clone, Debug)]
pub struct MouseSettings {
    /// KF's MouseSensitivity (0.25 to 25).
    pub sensitivity: f32,
    /// KF's bInvertMouse.
    pub invert: bool,
    /// The settings file (`--settings FILE`, else the launcher's).
    pub path: std::path::PathBuf,
    /// The sensitivity when a slider drag started (saved at its end).
    drag_start: Option<f32>,
}

impl MouseSettings {
    /// Reads the saved settings; `--sensitivity` / `--invert-mouse` (if
    /// given) win for this run without being saved.
    pub fn load(path: std::path::PathBuf, sensitivity: Option<f32>, invert: Option<bool>) -> Self {
        let ((saved_sens, saved_inv), from) = crate::launcher::read_mouse(&path);
        let s = MouseSettings { sensitivity: sensitivity.map_or(saved_sens, choices::clamp_sensitivity), invert: invert.unwrap_or(saved_inv), path, drag_start: None };
        let read = if sensitivity.is_some() || invert.is_some() { "command_line" } else { from };
        runlog::kv(
            "mouse_settings",
            &format!(
                "{} source=start read={read} saved_sensitivity={saved_sens:.2} saved_invert={saved_inv} settings={} units_per_count={:.3} degrees_per_count_fov90={:.5}",
                s.describe(),
                s.path.display(),
                UNITS_PER_COUNT * s.sensitivity,
                radians_per_count(s.sensitivity, 1.0).to_degrees(),
            ),
        );
        s
    }

    /// `sensitivity=3.00 invert=false` (the log's words).
    pub fn describe(&self) -> String {
        format!("sensitivity={:.2} invert={}", self.sensitivity, self.invert)
    }

    /// Changes the sensitivity (clamped to KF's range) and saves; true if
    /// it changed.
    pub fn set_sensitivity(&mut self, v: f32, source: &str) -> bool {
        let v = choices::clamp_sensitivity(v);
        let changed = v != self.sensitivity;
        self.sensitivity = v;
        self.changed(changed, source)
    }

    /// A slider drag: changes the sensitivity at once (the next mouse
    /// movement uses it) but logs and saves only at `commit`, when the
    /// drag ends.
    pub fn drag_sensitivity(&mut self, v: f32) {
        self.drag_start.get_or_insert(self.sensitivity);
        self.sensitivity = choices::clamp_sensitivity(v);
    }

    /// The end of a slider drag: logs, and saves if the drag changed it.
    pub fn commit(&mut self, source: &str) -> bool {
        let changed = self.drag_start.take().is_some_and(|s| s != self.sensitivity);
        self.changed(changed, source)
    }

    /// Changes invert mouse and saves; true if it changed.
    pub fn set_invert(&mut self, invert: bool, source: &str) -> bool {
        let changed = invert != self.invert;
        self.invert = invert;
        self.changed(changed, source)
    }

    fn changed(&self, changed: bool, source: &str) -> bool {
        runlog::kv("mouse_settings", &format!("{} source={source} changed={changed}", self.describe()));
        if changed {
            match crate::launcher::save_mouse(&self.path, self.sensitivity, self.invert) {
                Ok(()) => runlog::kv("mouse_saved", &format!("file={} {}", self.path.display(), self.describe())),
                Err(e) => runlog::kv("mouse_save_failed", &format!("file={} reason=\"{e}\"", self.path.display())),
            }
        }
        changed
    }
}

impl Default for MouseSettings {
    /// KF's defaults, saved to the launcher's file (tests insert their own).
    fn default() -> Self {
        MouseSettings { sensitivity: choices::SENSITIVITY_DEFAULT, invert: false, path: crate::launcher::SETTINGS_PATH.into(), drag_start: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_turn_as_kf() {
        // 12.8 units per count at sensitivity 1, FOV 90: 0.0703125 degrees.
        assert!((UNITS_PER_COUNT - 12.8).abs() < 1e-6);
        assert!((radians_per_count(1.0, 1.0).to_degrees() - 0.0703125).abs() < 1e-6);
        // KF's default 3 at FOV 90: 38.4 units, 0.2109 degrees per count.
        assert!((radians_per_count(3.0, fov_scale(90.0, false)).to_degrees() - 0.2109375).abs() < 1e-5);
        // The old fixed 0.002 radians per count is sensitivity 1.63.
        assert!((0.002 / radians_per_count(1.0, 1.0) - 1.6297).abs() < 1e-3);
        // Iron sights at FOV 45 halve it; a drawn 3D scope uses 24.
        assert!((fov_scale(45.0, false) - 0.5).abs() < 1e-6);
        assert!((fov_scale(45.0, true) - 24.0 / 90.0).abs() < 1e-6);
    }

    #[test]
    fn look_directions_and_invert() {
        let r = radians_per_count(3.0, 1.0);
        // Mouse right: yaw falls (turns right); mouse forward (y < 0): looks up.
        let (yaw, pitch) = look_delta(Vec2::new(10.0, -4.0), 3.0, false, 1.0);
        assert!((yaw + 10.0 * r).abs() < 1e-7 && (pitch - 4.0 * r).abs() < 1e-7);
        // Inverted: only up / down flips.
        let (yaw_i, pitch_i) = look_delta(Vec2::new(10.0, -4.0), 3.0, true, 1.0);
        assert!((yaw_i - yaw).abs() < 1e-9 && (pitch_i + pitch).abs() < 1e-9);
        // Twice the sensitivity, twice the turn.
        let (yaw2, _) = look_delta(Vec2::new(10.0, 0.0), 6.0, false, 1.0);
        assert!((yaw2 - 2.0 * yaw).abs() < 1e-7);
    }

    #[test]
    fn settings_load_override_and_save() {
        let dir = std::env::temp_dir().join(format!("openkf-mouse-test-{}", std::process::id()));
        let path = dir.join("launcher.txt");
        let _ = std::fs::remove_dir_all(&dir);
        // No file: KF's defaults.
        let mut s = MouseSettings::load(path.clone(), None, None);
        assert_eq!((s.sensitivity, s.invert), (3.0, false));
        // Changing saves only the mouse lines.
        assert!(s.set_sensitivity(1.5, "test"));
        assert!(!s.set_sensitivity(1.5, "test"));
        assert!(s.set_invert(true, "test"));
        // A drag saves once, when it ends.
        s.drag_sensitivity(4.0);
        s.drag_sensitivity(1.5);
        assert!(!s.commit("test"));
        s.drag_sensitivity(2.0);
        assert!(crate::launcher::read_mouse(&path).0 == (1.5, true));
        assert!(s.commit("test"));
        assert!(crate::launcher::read_mouse(&path).0 == (2.0, true));
        s.set_sensitivity(1.5, "test");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("mouse_sensitivity=1.50") && text.contains("invert_mouse=on"), "{text}");
        assert_eq!(MouseSettings::load(path.clone(), None, None).sensitivity, 1.5);
        // The command line wins for the run; the file keeps its value.
        let s = MouseSettings::load(path.clone(), Some(9.0), Some(false));
        assert_eq!((s.sensitivity, s.invert), (9.0, false));
        assert_eq!(crate::launcher::read_mouse(&path).0, (1.5, true));
        // Out of range values are clamped to KF's box.
        let mut s = MouseSettings::load(path.clone(), None, None);
        s.set_sensitivity(100.0, "test");
        assert_eq!(s.sensitivity, choices::SENSITIVITY_MAX);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
