use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SNAPSHOT_LIMIT: u64 = 2 * 1024 * 1024;
const KEEP_FOR: Duration = Duration::from_secs(14 * 24 * 60 * 60);
pub const LOG_FILE: &str = "log.jsonl";

pub fn hash_bytes(parts: &[&[u8]]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for &b in part.iter().chain(std::iter::once(&0xff)) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

pub fn hash(parts: &[&str]) -> String {
    hash_bytes(&parts.iter().map(|p| p.as_bytes()).collect::<Vec<_>>())
}

pub fn content_hash(file: &Path) -> String {
    fs::read(file).map_or_else(|_| "missing".into(), |b| hash_bytes(&[&b]))
}

pub struct State {
    pub dir: PathBuf,
    pub snapshot: PathBuf,
    pub blocks: PathBuf,
    pub passed: PathBuf,
    pub noticed: PathBuf,
}

impl State {
    pub fn new(base: PathBuf, session: &str, root: &Path) -> Self {
        let key = root.to_string_lossy();
        let session_key = hash(&[session, &key]);
        let _ = fs::create_dir_all(&base);
        State {
            snapshot: base.join(format!("snapshot-{session_key}")),
            blocks: base.join(format!("blocks-{session_key}")),
            noticed: base.join(format!("noticed-{session_key}")),
            passed: base.join(format!("passed-{}", hash(&[&key]))),
            dir: base,
        }
    }

    pub fn base_dir() -> PathBuf {
        std::env::var("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state")
            })
            .join("kiln")
    }

    pub fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    pub fn save_snapshot(&self, root: &Path, files: &[PathBuf]) {
        let _ = fs::remove_dir_all(&self.snapshot);
        let copies = self.snapshot.join("files");
        let _ = fs::create_dir_all(&copies);
        let mut index = String::new();
        for f in files {
            let digest = content_hash(f);
            let small = fs::metadata(f).is_ok_and(|m| m.is_file() && m.len() <= SNAPSHOT_LIMIT);
            if small {
                let _ = fs::copy(f, copies.join(&digest));
            }
            let rel = f.strip_prefix(root).unwrap_or(f);
            index.push_str(&format!("{digest}\t{}\n", rel.display()));
        }
        let _ = fs::write(self.snapshot.join("index"), index);
    }

    pub fn load_snapshot(&self, root: &Path) -> Option<HashMap<PathBuf, String>> {
        let index = fs::read_to_string(self.snapshot.join("index")).ok()?;
        Some(
            index
                .lines()
                .filter_map(|l| l.split_once('\t'))
                .map(|(digest, rel)| (root.join(rel), digest.to_string()))
                .collect(),
        )
    }

    pub fn snapshot_copy(&self, digest: &str) -> Option<PathBuf> {
        let path = self.snapshot.join("files").join(digest);
        path.is_file().then_some(path)
    }

    pub fn clean_old(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let now = SystemTime::now();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == LOG_FILE) {
                continue;
            }
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| now.duration_since(t).unwrap_or_default() > KEEP_FOR);
            if old {
                let _ = fs::remove_dir_all(&path).or_else(|_| fs::remove_file(&path));
            }
        }
    }

    pub fn log(&self, entry: Value) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut entry = entry;
        entry["ts"] = json!(ts);
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(LOG_FILE))
        {
            let _ = writeln!(f, "{entry}");
        }
    }
}
