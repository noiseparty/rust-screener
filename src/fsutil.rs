//! File helpers shared by the caches and the config.

use std::fs;
use std::io;
use std::path::Path;

/// Writes `data` as the whole of `path` via a `.tmp` file and a rename, so a crash never
/// leaves a half-written file. Creates the parent directory.
pub fn write_atomic(path: &Path, data: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(tmp, path)
}

#[cfg(test)]
pub(crate) mod test {
    use std::fs;
    use std::path::PathBuf;

    /// A fresh, empty scratch directory `target/<group>/<name>`.
    pub fn scratch(group: &str, name: &str) -> PathBuf {
        let d = PathBuf::from("target").join(group).join(name);
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_tmp() {
        let d = scratch("test-fsutil", "atomic");
        let p = d.join("sub").join("f.csv");
        super::write_atomic(&p, "a").unwrap();
        super::write_atomic(&p, "b").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "b");
        assert!(!p.with_extension("tmp").exists());
    }
}
