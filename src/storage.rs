//! Config -> Storage: a small file browser over the app home, to see how much ticker data the
//! app keeps and delete what is no longer needed. Nothing is capped or deleted automatically
//! here (old candle files and 4h caches unused for 30 days are cleaned elsewhere).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Files the browser never deletes.
const PROTECTED: [&str; 1] = ["config.json"];

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub dir: bool,
    /// File size, or everything under a folder.
    pub bytes: u64,
    pub modified: Option<SystemTime>,
}

/// Bytes of everything under `path` (0 when it is missing).
pub fn size(path: &Path) -> u64 {
    let Ok(meta) = fs::metadata(path) else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path).map_or(0, |rd| rd.flatten().map(|e| size(&e.path())).sum())
}

/// The entries of `dir`: folders first by name, then files largest first.
pub fn list(dir: &Path) -> io::Result<Vec<Entry>> {
    let mut v: Vec<Entry> = fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let path = e.path();
            Some(Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                bytes: if meta.is_dir() { size(&path) } else { meta.len() },
                dir: meta.is_dir(),
                modified: meta.modified().ok(),
                path,
            })
        })
        .collect();
    v.sort_by(|a, b| b.dir.cmp(&a.dir).then(if a.dir { a.name.cmp(&b.name) } else { b.bytes.cmp(&a.bytes) }));
    Ok(v)
}

/// Deletes a file, or a folder with everything in it. `config.json` is refused.
pub fn delete(e: &Entry) -> io::Result<()> {
    if PROTECTED.contains(&e.name.as_str()) {
        return Err(io::Error::other(format!("{} is the app's settings and is kept", e.name)));
    }
    if e.dir { fs::remove_dir_all(&e.path) } else { fs::remove_file(&e.path) }
}

/// `1.4 MB`, `820 KB`, `12 B`.
pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.0} KB", b / 1024.0),
        1_048_576..1_073_741_824 => format!("{:.1} MB", b / 1_048_576.0),
        _ => format!("{:.2} GB", b / 1_073_741_824.0),
    }
}

/// Browser state: the folder shown (never above `root`), the cursor and a pending delete.
pub struct Browser {
    pub root: PathBuf,
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    /// Waiting for y/n on deleting the entry under the cursor.
    pub confirm: bool,
    pub msg: Option<String>,
}

impl Browser {
    pub fn new(root: &Path) -> Browser {
        let mut b = Browser { root: root.to_path_buf(), cwd: root.to_path_buf(), entries: Vec::new(), cursor: 0, confirm: false, msg: None };
        b.refresh();
        b
    }

    pub fn refresh(&mut self) {
        match list(&self.cwd) {
            Ok(v) => self.entries = v,
            Err(e) => {
                self.entries.clear();
                self.msg = Some(format!("cannot read {}: {e}", self.cwd.display()));
            }
        }
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    pub fn move_by(&mut self, d: isize) {
        let n = self.entries.len();
        if n > 0 {
            self.cursor = (self.cursor as isize + d).clamp(0, n as isize - 1) as usize;
        }
        self.confirm = false;
    }

    /// Opens the folder under the cursor.
    pub fn enter(&mut self) {
        if let Some(e) = self.selected().filter(|e| e.dir) {
            self.cwd = e.path.clone();
            self.cursor = 0;
            self.msg = None;
            self.refresh();
        }
    }

    /// Goes to the parent folder; false at the root.
    pub fn up(&mut self) -> bool {
        if self.cwd == self.root {
            return false;
        }
        let from = self.cwd.clone();
        if let Some(p) = self.cwd.parent() {
            self.cwd = p.to_path_buf();
        }
        self.msg = None;
        self.refresh();
        self.cursor = self.entries.iter().position(|e| e.path == from).unwrap_or(0);
        true
    }

    /// Deletes the entry under the cursor (after `confirm`).
    pub fn delete_selected(&mut self) {
        self.confirm = false;
        let Some(e) = self.selected().cloned() else { return };
        self.msg = Some(match delete(&e) {
            Ok(()) => format!("deleted {} ({})", e.name, human(e.bytes)),
            Err(err) => format!("could not delete {}: {err}", e.name),
        });
        self.refresh();
    }

    /// The folder shown, relative to the root (`.` at the root).
    pub fn rel(&self) -> String {
        match self.cwd.strip_prefix(&self.root) {
            Ok(p) if p.as_os_str().is_empty() => ".".to_string(),
            Ok(p) => p.display().to_string(),
            Err(_) => self.cwd.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vs-storage-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("data/levels")).unwrap();
        fs::write(d.join("config.json"), "{}").unwrap();
        fs::write(d.join("data/candles-2026-11-17.csv"), vec![b'x'; 3000]).unwrap();
        fs::write(d.join("data/levels/AAPL.csv"), vec![b'x'; 500]).unwrap();
        fs::write(d.join("data/levels/GP.csv"), vec![b'x'; 900]).unwrap();
        d
    }

    #[test]
    fn sizes_and_listing_order() {
        let d = tree("list");
        assert_eq!(size(&d), 2 + 3000 + 500 + 900);
        assert_eq!(size(&d.join("missing")), 0);
        let v = list(&d.join("data")).unwrap();
        let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["levels", "candles-2026-11-17.csv"]);
        assert_eq!(v[0].bytes, 1400);
        let v = list(&d.join("data/levels")).unwrap();
        assert_eq!(v[0].name, "GP.csv");
        assert_eq!(human(512), "512 B");
        assert_eq!(human(2048), "2 KB");
        assert_eq!(human(3 * 1_048_576), "3.0 MB");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn browse_delete_and_stay_inside_the_root() {
        let d = tree("browse");
        let mut b = Browser::new(&d);
        assert_eq!(b.rel(), ".");
        assert!(!b.up());
        // config.json is listed but kept
        b.cursor = b.entries.iter().position(|e| e.name == "config.json").unwrap();
        b.delete_selected();
        assert!(d.join("config.json").exists());
        assert!(b.msg.as_deref().unwrap().contains("kept"));
        b.cursor = b.entries.iter().position(|e| e.name == "data").unwrap();
        b.enter();
        b.enter(); // "levels" is first
        assert_eq!(b.rel(), Path::new("data").join("levels").display().to_string());
        b.delete_selected(); // GP.csv, the largest
        assert!(!d.join("data/levels/GP.csv").exists());
        assert_eq!(b.entries.len(), 1);
        assert!(b.up());
        assert_eq!(b.selected().unwrap().name, "levels");
        // a whole folder
        b.delete_selected();
        assert!(!d.join("data/levels").exists());
        fs::remove_dir_all(&d).unwrap();
    }
}
