use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

const MAX_BLOCKS: u32 = 3;
const TAIL_LINES: usize = 40;

fn git(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn hash(parts: &[&str]) -> String {
    let mut h = DefaultHasher::new();
    parts.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn changed(root: &Path) -> Vec<PathBuf> {
    git(
        root,
        &[
            "status",
            "--porcelain",
            "-z",
            "--no-renames",
            "--untracked-files=all",
        ],
    )
    .split('\0')
    .filter(|e| e.len() > 3)
    .map(|e| root.join(&e[3..]))
    .collect()
}

fn fingerprint(root: &Path, files: &[PathBuf]) -> String {
    let mut lines = vec![git(root, &["rev-parse", "HEAD"])];
    for f in files {
        let stamp = fs::metadata(f)
            .map(|m| format!("{:?}:{}", m.modified().ok(), m.len()))
            .unwrap_or_default();
        lines.push(format!("{}:{}", f.display(), stamp));
    }
    lines.sort();
    hash(&lines.iter().map(String::as_str).collect::<Vec<_>>())
}

fn check_scripts(root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for f in files {
        for dir in f.ancestors().skip(1) {
            let script = dir.join("scripts/check.sh");
            if script.is_file() {
                if !found.contains(&script) {
                    found.push(script);
                }
                break;
            }
            if dir == root {
                break;
            }
        }
    }
    found
}

fn main() {
    let mode = env::args().nth(1).unwrap_or_default();
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let cwd = payload["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_default();
    let session = payload["session_id"].as_str().unwrap_or("unknown");

    let top = git(&cwd, &["rev-parse", "--show-toplevel"]);
    if top.trim().is_empty() {
        return;
    }
    let root = PathBuf::from(top.trim());
    let files = changed(&root);
    let now = fingerprint(&root, &files);

    let state = env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env::var("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("dev-standard");
    let _ = fs::create_dir_all(&state);
    let key = root.to_string_lossy();
    let baseline = state.join(format!("baseline-{}", hash(&[session, &key])));
    let blocks = state.join(format!("blocks-{}", hash(&[session, &key])));
    let passed = state.join(format!("passed-{}", hash(&[&key])));
    let read = |p: &Path| fs::read_to_string(p).unwrap_or_default();

    if mode == "start" {
        let _ = fs::write(&baseline, &now);
        return;
    }
    if read(&baseline) == now || read(&passed) == now {
        return;
    }

    let mut failures = Vec::new();
    for script in check_scripts(&root, &files) {
        let dir = script.parent().and_then(Path::parent).unwrap_or(&root);
        let Ok(out) = Command::new("bash")
            .arg(&script)
            .arg("--quick")
            .current_dir(dir)
            .output()
        else {
            continue;
        };
        if !out.status.success() {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            let lines: Vec<&str> = text.trim().lines().collect();
            let tail = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
            failures.push(format!("$ bash {} --quick\n{}", script.display(), tail));
        }
    }

    if failures.is_empty() {
        let _ = fs::write(&passed, fingerprint(&root, &changed(&root)));
        let _ = fs::remove_file(&blocks);
        return;
    }

    let count = read(&blocks).trim().parse::<u32>().unwrap_or(0) + 1;
    if count > MAX_BLOCKS {
        let _ = fs::remove_file(&blocks);
        let message = format!("표준 검사가 {MAX_BLOCKS}번 연속 실패한 채 답변이 끝났습니다");
        println!("{}", json!({ "systemMessage": message }));
        return;
    }
    let _ = fs::write(&blocks, count.to_string());
    let reason = format!(
        "표준 검사가 실패했습니다. 원인을 고친 뒤 끝내세요. 검사를 끄거나 규칙을 무시하는 표시로 넘기지 마세요. 이 세션이 만든 문제가 아니면 고치지 말고 그 사실을 사용자에게 알리세요.\n\n{}",
        failures.join("\n\n")
    );
    println!("{}", json!({ "decision": "block", "reason": reason }));
}
