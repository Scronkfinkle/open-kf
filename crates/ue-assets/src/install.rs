//! Finds the Killing Floor install on disk. The install is only ever read.

use std::fmt;
use std::path::{Path, PathBuf};

/// Environment variable that overrides install discovery.
pub const ROOT_ENV: &str = "KF_ROOT";

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
    /// No candidate path held a valid install.
    NotFound(Vec<PathBuf>),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::BadOverride(p) => write!(
                f,
                "{ROOT_ENV}={} does not contain System/Build.ini",
                p.display()
            ),
            InstallError::NotFound(tried) => {
                write!(f, "Killing Floor install not found. Tried:")?;
                for p in tried {
                    write!(f, "\n  {}", p.display())?;
                }
                write!(f, "\nSet {ROOT_ENV} to the install folder.")
            }
        }
    }
}

impl std::error::Error for InstallError {}

impl Install {
    /// Checks `KF_ROOT` first, then `references/killing_floor` in the current
    /// directory, then the usual Steam library locations.
    pub fn discover() -> Result<Install, InstallError> {
        if let Some(root) = std::env::var_os(ROOT_ENV) {
            let root = PathBuf::from(root);
            return Install::open(&root).ok_or(InstallError::BadOverride(root));
        }
        let candidates = default_candidates();
        candidates
            .iter()
            .find_map(|p| Install::open(p))
            .ok_or(InstallError::NotFound(candidates))
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

fn default_candidates() -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from("references/killing_floor")];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for steam in [".steam/steam", ".local/share/Steam"] {
            out.push(home.join(steam).join("steamapps/common/KillingFloor"));
        }
    }
    out.push(PathBuf::from(
        r"C:\Program Files (x86)\Steam\steamapps\common\KillingFloor",
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_reads_build_label() {
        let dir = std::env::temp_dir().join(format!("kf-rs-install-test-{}", std::process::id()));
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
