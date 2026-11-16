//! App home directory: `%LOCALAPPDATA%\VolumeScreener\` holding `config.json`,
//! `data\` and `probe\`. Base directories are passed in so the logic is testable.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const APP_DIR_NAME: &str = "VolumeScreener";
pub const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub probe: PathBuf,
}

impl Paths {
    /// Layout under `<local_base>\VolumeScreener`. Does not touch the disk.
    pub fn under(local_base: &Path) -> Paths {
        let home = local_base.join(APP_DIR_NAME);
        Paths {
            config: home.join(CONFIG_FILE),
            data: home.join("data"),
            probe: home.join("probe"),
            home,
        }
    }

    /// Layout under `%LOCALAPPDATA%`.
    pub fn from_env() -> io::Result<Paths> {
        let base = std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))?;
        Ok(Paths::under(Path::new(&base)))
    }

    /// Create the home, `data\` and `probe\` directories if missing.
    pub fn ensure_dirs(&self) -> io::Result<()> {
        fs::create_dir_all(&self.data)?;
        fs::create_dir_all(&self.probe)
    }

    /// First-run setup: create the directories, then move `<cwd>/config.json` into the
    /// app home if it exists there and the app home has none. Returns true if moved.
    pub fn init(&self, cwd: &Path) -> io::Result<bool> {
        self.ensure_dirs()?;
        migrate_config(&cwd.join(CONFIG_FILE), &self.config)
    }
}

/// Move `old` to `new` when `old` exists and `new` does not. Returns true if moved.
pub fn migrate_config(old: &Path, new: &Path) -> io::Result<bool> {
    if new.exists() || !old.is_file() {
        return Ok(false);
    }
    if let Some(parent) = new.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::rename(old, new).is_err() {
        // Different volume: copy then remove.
        fs::copy(old, new)?;
        fs::remove_file(old)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh scratch dir under the project's `target/` folder.
    fn scratch(name: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-paths").join(name);
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn layout_is_under_volume_screener() {
        let p = Paths::under(Path::new("base"));
        assert_eq!(p.home, Path::new("base").join("VolumeScreener"));
        assert_eq!(p.config, p.home.join("config.json"));
        assert_eq!(p.data, p.home.join("data"));
        assert_eq!(p.probe, p.home.join("probe"));
    }

    #[test]
    fn init_creates_dirs() {
        let root = scratch("dirs");
        let p = Paths::under(&root.join("local"));
        assert!(!p.home.exists());
        assert!(!p.init(&root.join("cwd")).unwrap());
        assert!(p.data.is_dir() && p.probe.is_dir());
        assert!(!p.config.exists());
    }

    #[test]
    fn moves_cwd_config_on_first_run() {
        let root = scratch("move");
        let cwd = root.join("cwd");
        fs::create_dir_all(&cwd).unwrap();
        fs::write(cwd.join("config.json"), "{\"a\":1}").unwrap();
        let p = Paths::under(&root.join("local"));
        assert!(p.init(&cwd).unwrap());
        assert_eq!(fs::read_to_string(&p.config).unwrap(), "{\"a\":1}");
        assert!(!cwd.join("config.json").exists());
        // Second run is a no-op.
        assert!(!p.init(&cwd).unwrap());
    }

    #[test]
    fn does_not_overwrite_existing_app_home_config() {
        let root = scratch("keep");
        let cwd = root.join("cwd");
        fs::create_dir_all(&cwd).unwrap();
        fs::write(cwd.join("config.json"), "old").unwrap();
        let p = Paths::under(&root.join("local"));
        p.ensure_dirs().unwrap();
        fs::write(&p.config, "new").unwrap();
        assert!(!p.init(&cwd).unwrap());
        assert_eq!(fs::read_to_string(&p.config).unwrap(), "new");
        assert_eq!(fs::read_to_string(cwd.join("config.json")).unwrap(), "old");
    }
}
