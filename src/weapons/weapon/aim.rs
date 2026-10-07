//! The aim button's mode: Toggle (KF's default binding, RightMouse =
//! ToggleAiming = ToggleIronSights) or Hold (KF's `Aiming` alias:
//! IronSightZoomIn, onrelease IronSightZoomOut). Saved as `aim=` in the
//! launcher's settings file; the pause menu changes it at once. See
//! DESIGN.md, "Aim down sights".

use bevy::prelude::*;

use crate::engine::runlog;

/// The aim mode in use and where it is saved.
#[derive(Resource, Clone, Debug)]
pub struct AimSetting {
    /// Aim only while the button is held (else a press toggles).
    pub hold: bool,
    /// The settings file (`--settings FILE`, else the launcher's).
    pub path: std::path::PathBuf,
}

impl AimSetting {
    /// Reads the saved mode (KF's toggle if there is none).
    pub fn load(path: std::path::PathBuf) -> Self {
        let (hold, from) = crate::launcher::read_aim(&path);
        runlog::kv("aim_mode", &format!("mode={} source=start settings={} read={from}", Self::word(hold), path.display()));
        AimSetting { hold, path }
    }

    pub fn word(hold: bool) -> &'static str {
        crate::launcher::choices::aim_word(hold)
    }

    pub fn mode(&self) -> &'static str {
        Self::word(self.hold)
    }

    /// Changes the mode (if different) and saves it; true if it changed.
    pub fn set(&mut self, hold: bool, source: &str) -> bool {
        if hold == self.hold {
            runlog::kv("aim_mode", &format!("mode={} source={source} changed=false", self.mode()));
            return false;
        }
        self.hold = hold;
        runlog::kv("aim_mode", &format!("mode={} source={source} changed=true", self.mode()));
        match crate::launcher::save_aim(&self.path, hold) {
            Ok(()) => runlog::kv("aim_saved", &format!("file={} aim={}", self.path.display(), self.mode())),
            Err(e) => runlog::kv("aim_save_failed", &format!("file={} reason=\"{e}\"", self.path.display())),
        }
        true
    }
}

impl Default for AimSetting {
    /// KF's toggle, saved to the launcher's file (tests insert their own).
    fn default() -> Self {
        AimSetting { hold: false, path: crate::launcher::SETTINGS_PATH.into() }
    }
}
