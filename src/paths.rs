//! App home directory: `%LOCALAPPDATA%\MarketGUI\` holding `config.json`,
//! `data\`, `probe\` and `logs\` (trace mode). Base directories are passed in so the logic is testable.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const APP_DIR_NAME: &str = "MarketGUI";
/// The app home's name before the rename to MarketGUI.
pub const OLD_APP_DIR_NAME: &str = "VolumeScreener";
pub const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub probe: PathBuf,
    /// Trace logs (`trace`).
    pub logs: PathBuf,
}

impl Paths {
    /// Layout under `<local_base>\MarketGUI`. Does not touch the disk.
    pub fn under(local_base: &Path) -> Paths {
        Paths::at(local_base.join(APP_DIR_NAME))
    }

    /// Layout with `home` as the app home.
    pub fn at(home: PathBuf) -> Paths {
        Paths { config: home.join(CONFIG_FILE), data: home.join("data"), probe: home.join("probe"), logs: home.join("logs"), home }
    }

    /// Layout under `%LOCALAPPDATA%`.
    pub fn from_env() -> io::Result<Paths> {
        let base = std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))?;
        Ok(Paths::under(Path::new(&base)))
    }

    /// Create the home, `data\`, `probe\` and `logs\` directories if missing.
    pub fn ensure_dirs(&self) -> io::Result<()> {
        fs::create_dir_all(&self.data)?;
        fs::create_dir_all(&self.probe)?;
        fs::create_dir_all(&self.logs)
    }

    /// The old app home (`VolumeScreener`) next to `home`.
    pub fn old_home(&self) -> Option<PathBuf> {
        self.home.parent().map(|p| p.join(OLD_APP_DIR_NAME))
    }

    /// One-time rename of the old app home to `home`, when the old folder exists and `home` is
    /// missing or holds no files at all (e.g. left empty by an earlier launch whose rename failed;
    /// its empty folders are removed first). Returns true if renamed. Nothing with files is touched.
    pub fn migrate_home(&self) -> io::Result<bool> {
        let Some(old) = self.old_home() else { return Ok(false) };
        if !old.is_dir() || (self.home.exists() && !remove_empty_tree(&self.home)?) {
            return Ok(false);
        }
        fs::rename(&old, &self.home)?;
        Ok(true)
    }

    /// First-run setup: create the directories, then move `<cwd>/config.json` into the
    /// app home if it exists there and the app home has none. Returns true if moved.
    pub fn init(&self, cwd: &Path) -> io::Result<bool> {
        self.ensure_dirs()?;
        migrate_config(&cwd.join(CONFIG_FILE), &self.config)
    }
}

/// Removes `dir` if it and its subfolders hold no files. Returns whether it was removed.
fn remove_empty_tree(dir: &Path) -> io::Result<bool> {
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if !p.is_dir() || !remove_empty_tree(&p)? {
            return Ok(false);
        }
    }
    fs::remove_dir(dir)?;
    Ok(true)
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
    fn layout_is_under_market_gui() {
        let p = Paths::under(Path::new("base"));
        assert_eq!(p.home, Path::new("base").join("MarketGUI"));
        assert_eq!(p.config, p.home.join("config.json"));
        assert_eq!(p.data, p.home.join("data"));
        assert_eq!(p.probe, p.home.join("probe"));
        assert_eq!(p.logs, p.home.join("logs"));
    }

    #[test]
    fn renames_old_home_once() {
        let root = scratch("migrate");
        let old = root.join("VolumeScreener");
        fs::create_dir_all(old.join("data")).unwrap();
        fs::write(old.join("config.json"), "kept").unwrap();
        let p = Paths::under(&root);
        assert!(p.migrate_home().unwrap());
        assert!(!old.exists());
        assert_eq!(fs::read_to_string(&p.config).unwrap(), "kept");
        assert!(p.data.is_dir());
        // the new home holds files now: a recreated old folder is left alone
        fs::create_dir_all(&old).unwrap();
        assert!(!p.migrate_home().unwrap());
        assert!(old.is_dir() && p.config.exists());
        // a new home with only empty folders (an earlier failed rename) is replaced
        let root2 = scratch("migrate-empty");
        let old2 = root2.join(OLD_APP_DIR_NAME);
        fs::create_dir_all(&old2).unwrap();
        fs::write(old2.join("config.json"), "kept").unwrap();
        let p2 = Paths::under(&root2);
        fs::create_dir_all(p2.data.join("candles")).unwrap();
        assert!(p2.migrate_home().unwrap());
        assert_eq!(fs::read_to_string(&p2.config).unwrap(), "kept");
        // nothing to move
        assert!(!Paths::under(&root.join("empty")).migrate_home().unwrap());
    }

    #[test]
    fn init_creates_dirs() {
        let root = scratch("dirs");
        let p = Paths::under(&root.join("local"));
        assert!(!p.home.exists());
        assert!(!p.init(&root.join("cwd")).unwrap());
        assert!(p.data.is_dir() && p.probe.is_dir() && p.logs.is_dir());
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
