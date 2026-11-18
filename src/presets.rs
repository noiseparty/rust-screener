//! Config -> Presets: named Configuration settings, saved as `<home>/presets/<name>.json` (a whole
//! `Config`; only the Configuration fields are taken when one is loaded). "Recommended" is built in.

use crate::config::Config;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The built-in preset (`Config::recommended`), listed first and never deleted.
pub const BUILTIN: &str = "Recommended";
/// Longest preset name.
pub const NAME_MAX: usize = 24;

/// The presets folder under the app home.
pub fn dir(home: &Path) -> PathBuf {
    home.join("presets")
}

fn file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

/// `name` kept to letters, digits, space, `_` and `-`, trimmed, at most `NAME_MAX` characters;
/// None when nothing is left or it is a Windows device name (CON, COM1, ...).
pub fn sanitize(name: &str) -> Option<String> {
    let kept: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-')).collect();
    let kept: String = kept.trim().chars().take(NAME_MAX).collect();
    let kept = kept.trim_end().to_string();
    // Windows device names cannot be file names
    let stem = kept.to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    (!kept.is_empty() && !reserved).then_some(kept)
}

/// Whether `name` is the built-in preset (any letter case).
pub fn is_builtin(name: &str) -> bool {
    name.eq_ignore_ascii_case(BUILTIN)
}

/// The built-in preset, then the saved ones by name (a missing folder lists none).
pub fn list(dir: &Path) -> Vec<String> {
    let mut saved: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let stem = p.file_stem()?.to_str()?.to_string();
                    (p.extension()? == "json" && !is_builtin(&stem)).then_some(stem)
                })
                .collect()
        })
        .unwrap_or_default();
    saved.sort_by_key(|s| s.to_lowercase());
    std::iter::once(BUILTIN.to_string()).chain(saved).collect()
}

pub fn exists(dir: &Path, name: &str) -> bool {
    is_builtin(name) || file(dir, name).exists()
}

/// The preset's config; the built-in one is `Config::recommended(base)`.
pub fn load(dir: &Path, name: &str, base: &Config) -> io::Result<Config> {
    if is_builtin(name) {
        return Ok(Config::recommended(base));
    }
    let path = file(dir, name);
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("no preset {name}")));
    }
    Config::load(&path)
}

/// Saves `cfg` as preset `name` (replacing one of that name); the built-in name is refused.
pub fn save(dir: &Path, name: &str, cfg: &Config) -> io::Result<()> {
    if is_builtin(name) {
        return Err(io::Error::other(format!("{BUILTIN} is built in")));
    }
    fs::create_dir_all(dir)?;
    cfg.save(&file(dir, name))
}

/// Deletes preset `name`; the built-in one is refused.
pub fn delete(dir: &Path, name: &str) -> io::Result<()> {
    if is_builtin(name) {
        return Err(io::Error::other(format!("{BUILTIN} is built in")));
    }
    fs::remove_file(file(dir, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sanitised() {
        assert_eq!(sanitize("  my preset_1-a  ").as_deref(), Some("my preset_1-a"));
        assert_eq!(sanitize("a/b\\c:d*?.json").as_deref(), Some("abcdjson"));
        assert_eq!(sanitize("../.."), None);
        assert_eq!((sanitize("con"), sanitize("COM1"), sanitize("console").as_deref()), (None, None, Some("console")));
        assert_eq!(sanitize("   "), None);
        assert_eq!(sanitize(&"x".repeat(40)).unwrap().len(), NAME_MAX);
        // a cut that ends on a space leaves no trailing space
        assert_eq!(sanitize(&format!("{} b", "a".repeat(23))).as_deref(), Some("a".repeat(23).as_str()));
    }

    #[test]
    fn save_list_load_delete_round_trip() {
        let d = std::path::PathBuf::from("target").join("test-presets");
        let _ = fs::remove_dir_all(&d);
        assert_eq!(list(&d), vec![BUILTIN.to_string()]);
        let base = Config::default();
        let a = Config { min_price: 2.5, ..Config::default() };
        save(&d, "zeta", &a).unwrap();
        save(&d, "Alpha", &Config { max_price: 9.0, ..Config::default() }).unwrap();
        assert_eq!(list(&d), vec![BUILTIN.to_string(), "Alpha".into(), "zeta".into()]);
        assert!(exists(&d, "zeta") && exists(&d, "recommended") && !exists(&d, "nope"));
        assert_eq!(load(&d, "zeta", &base).unwrap().min_price, 2.5);
        assert_eq!(load(&d, BUILTIN, &base).unwrap(), Config::recommended(&base));
        assert!(load(&d, "nope", &base).is_err());
        // overwrite, and the built-in name is neither saved nor deleted
        save(&d, "zeta", &Config { min_price: 3.0, ..Config::default() }).unwrap();
        assert_eq!(load(&d, "zeta", &base).unwrap().min_price, 3.0);
        assert!(save(&d, "recommended", &a).is_err());
        assert!(delete(&d, BUILTIN).is_err());
        delete(&d, "zeta").unwrap();
        assert_eq!(list(&d), vec![BUILTIN.to_string(), "Alpha".into()]);
        fs::remove_dir_all(&d).unwrap();
    }
}
