use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
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

pub fn stamp(file: &Path) -> String {
    fs::metadata(file).map_or_else(
        |_| "missing".into(),
        |m| {
            let modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!("{}:{modified}", m.len())
        },
    )
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
    pub rules: PathBuf,
    pub touched: PathBuf,
    pub windows: PathBuf,
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
            rules: base.join(format!("rules-{session_key}")),
            touched: base.join(format!("touched-{session_key}")),
            windows: base.join(format!("windows-{session_key}")),
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

    pub fn open_window(&self, id: &str, root: &Path, files: &[PathBuf]) {
        let _ = fs::create_dir_all(&self.windows);
        let body: String = files
            .iter()
            .map(|f| {
                format!(
                    "{}\t{}\n",
                    stamp(f),
                    f.strip_prefix(root).unwrap_or(f).display()
                )
            })
            .collect();
        let _ = fs::write(self.windows.join(hash(&[id])), body);
        if !self.touched.exists() {
            let _ = fs::write(&self.touched, "");
        }
    }

    pub fn close_window(&self, id: &str, root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
        let path = self.windows.join(hash(&[id]));
        let Ok(body) = fs::read_to_string(&path) else {
            return Vec::new();
        };
        let _ = fs::remove_file(&path);
        let before: HashMap<PathBuf, &str> = body
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(s, rel)| (root.join(rel), s))
            .collect();
        files
            .iter()
            .filter(|f| before.get(*f).copied() != Some(stamp(f).as_str()))
            .cloned()
            .collect()
    }

    pub fn add_touched(&self, root: &Path, files: &[PathBuf]) {
        let lines: String = files
            .iter()
            .filter_map(|f| f.strip_prefix(root).ok())
            .map(|rel| format!("{}\n", rel.display()))
            .collect();
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.touched)
        {
            let _ = f.write_all(lines.as_bytes());
        }
    }

    pub fn load_touched(&self, root: &Path) -> Option<HashSet<PathBuf>> {
        let text = fs::read_to_string(&self.touched).ok()?;
        Some(text.lines().map(|rel| root.join(rel)).collect())
    }

    pub fn snapshot_copy(&self, digest: &str) -> Option<PathBuf> {
        let path = self.snapshot.join("files").join(digest);
        path.is_file().then_some(path)
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

fn roots_file(base: &Path, session: &str) -> PathBuf {
    base.join(format!("roots-{}", hash(&[session])))
}

pub fn session_roots(base: &Path, session: &str) -> Vec<PathBuf> {
    fs::read_to_string(roots_file(base, session))
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .collect()
}

pub fn add_session_root(base: &Path, session: &str, root: &Path) {
    if session_roots(base, session).iter().any(|r| r == root) {
        return;
    }
    let _ = fs::create_dir_all(base);
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(roots_file(base, session))
    {
        let _ = writeln!(f, "{}", root.display());
    }
}

pub fn clean_old(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
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

const NAMED_FOR: Duration = Duration::from_secs(24 * 60 * 60);

fn named_file(base: &Path, session: &str) -> PathBuf {
    base.join(format!("named-{}", hash(&[session])))
}

pub fn add_named(base: &Path, session: &str, files: &[PathBuf]) {
    if files.is_empty() {
        return;
    }
    let _ = fs::create_dir_all(base);
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(named_file(base, session))
    {
        for file in files {
            let _ = writeln!(f, "{}", file.display());
        }
    }
}

fn read_named(path: &Path) -> HashSet<PathBuf> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .collect()
}

pub fn named_only_by_others(base: &Path, session: &str) -> HashSet<PathBuf> {
    let own_path = named_file(base, session);
    let own = read_named(&own_path);
    let now = SystemTime::now();
    let mut others = HashSet::new();
    for entry in fs::read_dir(base).into_iter().flatten().flatten() {
        let path = entry.path();
        let is_named = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("named-"));
        let recent = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| now.duration_since(t).unwrap_or_default() < NAMED_FOR);
        if is_named && recent && path != own_path {
            others.extend(read_named(&path));
        }
    }
    others.retain(|f| !own.contains(f));
    others
}
