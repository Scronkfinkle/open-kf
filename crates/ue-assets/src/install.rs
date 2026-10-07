//! Finds the Killing Floor install on disk. The install is only ever read.

use std::fmt;
use std::path::{Path, PathBuf};

/// Environment variable that overrides install discovery.
pub const ROOT_ENV: &str = "KF_ROOT";

/// Killing Floor's Steam app id.
pub const STEAM_APP_ID: u32 = 1250;

/// Install used during development, relative to the current directory.
const DEV_ROOT: &str = "references/killing_floor";

/// A located Killing Floor install.
#[derive(Debug, Clone)]
pub struct Install {
    pub root: PathBuf,
    /// The `Label=` line from `System/Build.ini`, e.g. `UT2004_Build_[2004-11-11_10.48]`.
    pub build_label: String,
}

#[derive(Debug)]
pub enum InstallError {
    /// `KF_ROOT` was set but does not point at a valid install.
    BadOverride(PathBuf),
    /// No valid install: the paths tried, and what the Steam search found.
    NotFound { tried: Vec<PathBuf>, steam: Vec<String> },
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::BadOverride(p) => write!(
                f,
                "{ROOT_ENV}={} does not contain System/Build.ini",
                p.display()
            ),
            InstallError::NotFound { tried, steam } => {
                write!(f, "Killing Floor install not found. Tried:")?;
                for p in tried {
                    write!(f, "\n  {}", p.display())?;
                }
                for note in steam {
                    write!(f, "\n  Steam: {note}")?;
                }
                write!(f, "\nSet {ROOT_ENV} to the install folder.")
            }
        }
    }
}

impl std::error::Error for InstallError {}

impl Install {
    /// Checks `KF_ROOT` first, then `references/killing_floor` in the current
    /// directory, then every Steam install and library that has Killing Floor
    /// (found with steamlocate: native, Flatpak and Snap Steam on Linux, the
    /// registry on Windows, and library folders on other drives).
    pub fn discover() -> Result<Install, InstallError> {
        if let Some(root) = std::env::var_os(ROOT_ENV) {
            let root = PathBuf::from(root);
            return Install::open(&root).ok_or(InstallError::BadOverride(root));
        }
        let mut tried = vec![PathBuf::from(DEV_ROOT)];
        if let Some(install) = Install::open(&tried[0]) {
            return Ok(install);
        }
        let (steam_dirs, steam) = steam_candidates();
        for dir in steam_dirs {
            if let Some(install) = Install::open(&dir) {
                return Ok(install);
            }
            tried.push(dir);
        }
        Err(InstallError::NotFound { tried, steam })
    }

    /// Returns `Some` if `root` looks like a Killing Floor install.
    pub fn open(root: &Path) -> Option<Install> {
        let ini = std::fs::read_to_string(root.join("System").join("Build.ini")).ok()?;
        let build_label = ini
            .lines()
            .find_map(|l| l.trim().strip_prefix("Label="))
            .unwrap_or("unknown")
            .trim()
            .to_string();
        Some(Install {
            root: root.to_path_buf(),
            build_label,
        })
    }
}

/// Killing Floor folders in every Steam install found, plus notes on
/// Steam installs that do not have it (for the error message).
fn steam_candidates() -> (Vec<PathBuf>, Vec<String>) {
    let mut dirs = Vec::new();
    let mut notes = Vec::new();
    let steams = match steamlocate::locate_all() {
        Ok(steams) => steams,
        Err(e) => {
            notes.push(format!("not found ({e})"));
            return (dirs, notes);
        }
    };
    if steams.is_empty() {
        notes.push("not found".to_string());
    }
    for steam in steams {
        match steam.find_app(STEAM_APP_ID) {
            Ok(Some((app, library))) => dirs.push(library.resolve_app_dir(&app)),
            Ok(None) => notes.push(format!(
                "{}: Killing Floor (app {STEAM_APP_ID}) is not installed",
                steam.path().display()
            )),
            Err(e) => notes.push(format!("{}: {e}", steam.path().display())),
        }
    }
    (dirs, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_reads_build_label() {
        let dir = std::env::temp_dir().join(format!("open-kf-install-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("System")).unwrap();
        std::fs::write(
            dir.join("System/Build.ini"),
            "[BuildVersion]\r\nLabel=UT2004_Build_[2004-11-11_10.48]\r\n",
        )
        .unwrap();
        let install = Install::open(&dir).unwrap();
        assert_eq!(install.build_label, "UT2004_Build_[2004-11-11_10.48]");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn open_rejects_folder_without_build_ini() {
        assert!(Install::open(Path::new("/nonexistent/kf")).is_none());
    }
}
