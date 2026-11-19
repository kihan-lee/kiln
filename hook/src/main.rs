use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

const MAX_BLOCKS: u32 = 3;
const TAIL_LINES: usize = 40;
const COMMENT_LIMIT: usize = 20;
const DIRECTIVES: [&str; 8] = [
    "noqa", "type:", "ty:", "pragma", "fmt:", "ruff:", "pyright:", "mypy:",
];

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

fn projects(root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for f in files {
        for dir in f.ancestors().skip(1) {
            if dir.join("pyproject.toml").is_file() || dir.join("Cargo.toml").is_file() {
                if !found.iter().any(|d| d == dir) {
                    found.push(dir.to_path_buf());
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

fn commands(dir: &Path, files: &[PathBuf], venvs: &[PathBuf]) -> Vec<Vec<String>> {
    let mut cmds = Vec::new();
    let py = fs::read_to_string(dir.join("pyproject.toml")).unwrap_or_default();
    let text = |p: &Path| p.to_string_lossy().into_owned();
    let tool = |name: &str| {
        venvs
            .iter()
            .find(|v| v.join("bin").join(name).is_file())
            .map(|v| (text(&v.join("bin").join(name)), text(v)))
    };
    let sources: Vec<String> = files
        .iter()
        .filter(|f| f.extension().is_some_and(|x| x == "py") && f.is_file())
        .filter_map(|f| f.strip_prefix(dir).ok())
        .map(text)
        .collect();
    if let (true, false, Some((ruff, _))) =
        (py.contains("[tool.ruff"), sources.is_empty(), tool("ruff"))
    {
        let mut check = vec![ruff.clone(), "check".into(), "--force-exclude".into()];
        check.extend(sources.iter().cloned());
        cmds.push(check);
        if py.contains("[tool.ruff.format]") {
            let mut format = vec![ruff, "format".into(), "--check".into()];
            format.push("--force-exclude".into());
            format.extend(sources.iter().cloned());
            cmds.push(format);
        }
    }
    if let (true, Some((ty, venv))) = (py.contains("[tool.ty"), tool("ty")) {
        cmds.push(vec![ty, "check".into(), "--python".into(), venv]);
    }
    if dir.join("Cargo.toml").is_file() {
        let cargo = format!("{}/.cargo/bin/cargo", env::var("HOME").unwrap_or_default());
        cmds.push(vec![cargo.clone(), "fmt".into(), "--check".into()]);
        let clippy = ["clippy", "-q", "--", "-D", "warnings"];
        cmds.push(
            std::iter::once(cargo)
                .chain(clippy.map(String::from))
                .collect(),
        );
    }
    cmds
}

fn forbids_comments(dir: &Path) -> bool {
    ["pyproject.toml", "Cargo.toml"].iter().any(|name| {
        fs::read_to_string(dir.join(name))
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .skip_while(|l| !(l.starts_with('[') && l.ends_with("dev-standard]")))
            .skip(1)
            .take_while(|l| !l.starts_with('['))
            .filter_map(|l| l.split_once('='))
            .any(|(key, value)| key.trim() == "comments" && value.trim() == "false")
    })
}

fn added_lines(root: &Path, file: &Path) -> Option<Vec<usize>> {
    let name = file.to_string_lossy();
    if git(root, &["ls-files", "--", &name]).trim().is_empty() {
        return None;
    }
    let diff = git(root, &["diff", "HEAD", "-U0", "--", &name]);
    let mut lines = Vec::new();
    for header in diff.lines().filter(|l| l.starts_with("@@")) {
        let Some(plus) = header.split_whitespace().find(|p| p.starts_with('+')) else {
            continue;
        };
        let mut parts = plus[1..].split(',');
        let start: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let count: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
        lines.extend(start..start + count);
    }
    Some(lines)
}

fn python_marks(src: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = src.chars().collect();
    let mut found = Vec::new();
    let (mut i, mut line, mut seen_code) = (0, 1, false);
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c == '#' {
            let end = chars[i..]
                .iter()
                .position(|&x| x == '\n')
                .map_or(chars.len(), |p| i + p);
            found.push((line, chars[i..end].iter().collect()));
            i = end;
        } else if c == '"' || c == '\'' {
            let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
            let width = if triple { 3 } else { 1 };
            let (start_line, body_start) = (line, i + width);
            let mut j = body_start;
            let closed = loop {
                match chars.get(j) {
                    None => break false,
                    Some('\\') => {
                        if chars.get(j + 1) == Some(&'\n') {
                            line += 1;
                        }
                        j += 2;
                    }
                    Some('\n') if !triple => break false,
                    Some('\n') => {
                        line += 1;
                        j += 1;
                    }
                    Some(&x) if x == c => {
                        if !triple || (chars.get(j + 1) == Some(&c) && chars.get(j + 2) == Some(&c))
                        {
                            break true;
                        }
                        j += 1;
                    }
                    Some(_) => j += 1,
                }
            };
            let body_end = j.min(chars.len());
            let after = if closed { j + width } else { body_end };
            if !seen_code {
                let rest: String = chars[after..].iter().take_while(|&&x| x != '\n').collect();
                if rest.trim().is_empty() || rest.trim().starts_with('#') {
                    let body: String = chars[body_start..body_end].iter().collect();
                    let first: String = body
                        .trim()
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(60)
                        .collect();
                    found.push((start_line, format!("파일 맨 위 설명문: {first}")));
                }
            }
            seen_code = true;
            i = after;
        } else if c.is_whitespace() {
            i += 1;
        } else {
            let prefix = chars[i..]
                .iter()
                .take_while(|x| "rRuUbBfF".contains(**x))
                .count();
            if prefix > 0 && matches!(chars.get(i + prefix), Some('"') | Some('\'')) {
                i += prefix;
            } else {
                seen_code = true;
                i += 1;
            }
        }
    }
    found
}

fn comments(file: &Path) -> Vec<(usize, String)> {
    match file.extension().and_then(|x| x.to_str()) {
        Some("py") => python_marks(&fs::read_to_string(file).unwrap_or_default()),
        Some("rs") => fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l.trim().to_string()))
            .filter(|(_, l)| l.starts_with("//") && !l.starts_with("///") && !l.starts_with("//!"))
            .collect(),
        _ => Vec::new(),
    }
}

fn allowed(text: &str, line: usize) -> bool {
    let body = text.trim_start_matches(['#', '/']).trim_start();
    (line == 1 && text.starts_with("#!"))
        || body.starts_with("이유:")
        || DIRECTIVES.iter().any(|d| body.starts_with(d))
}

fn new_comments(root: &Path, dir: &Path, files: &[PathBuf]) -> Vec<String> {
    let mut found = Vec::new();
    for f in files.iter().filter(|f| f.is_file()) {
        let all = comments(f);
        if all.is_empty() {
            continue;
        }
        let added = added_lines(root, f);
        for (n, text) in all {
            if added.as_ref().is_none_or(|a| a.contains(&n)) && !allowed(&text, n) {
                let shown = f.strip_prefix(dir).unwrap_or(f).display();
                found.push(format!("{shown}:{n}: {text}"));
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

    let path = format!(
        "{}/.cargo/bin:{}/.local/bin:{}",
        env::var("HOME").unwrap_or_default(),
        env::var("HOME").unwrap_or_default(),
        env::var("PATH").unwrap_or_default()
    );
    let common = git(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let main_root = Path::new(common.trim())
        .parent()
        .unwrap_or(&root)
        .to_path_buf();
    let mut failures = Vec::new();
    for dir in projects(&root, &files) {
        let inside: Vec<PathBuf> = files
            .iter()
            .filter(|f| f.starts_with(&dir))
            .cloned()
            .collect();
        let mut venvs = vec![dir.join(".venv")];
        if let Ok(rel) = dir.strip_prefix(&root) {
            venvs.push(main_root.join(rel).join(".venv"));
        }
        if forbids_comments(&dir) {
            let found = new_comments(&root, &dir, &inside);
            if !found.is_empty() {
                let shown: Vec<&str> = found
                    .iter()
                    .take(COMMENT_LIMIT)
                    .map(String::as_str)
                    .collect();
                failures.push(format!(
                    "{}$ 새로 추가된 주석과 파일 맨 위 설명문 {}건\n{}\n주석 대신 이름, 타입, 테스트로 뜻을 드러내세요. 코드로 드러낼 수 없는 이유만 '이유:'로 시작하는 한 줄 주석으로 남길 수 있습니다. 파일 맨 위 설명문은 쓰지 않습니다.",
                    dir.display(),
                    found.len(),
                    shown.join("\n")
                ));
                continue;
            }
        }
        for cmd in commands(&dir, &inside, &venvs) {
            let Ok(out) = Command::new(&cmd[0])
                .args(&cmd[1..])
                .env("PATH", &path)
                .env("TERM", "dumb")
                .env("NO_COLOR", "1")
                .current_dir(&dir)
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
                let shown = cmd.join(" ").replace(&format!("{}/", dir.display()), "");
                failures.push(format!("{}$ {}\n{}", dir.display(), shown, tail));
                break;
            }
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

#[cfg(test)]
mod tests {
    use super::python_marks;

    fn lines(src: &str) -> Vec<usize> {
        python_marks(src).into_iter().map(|(n, _)| n).collect()
    }

    #[test]
    fn finds_comments_and_module_docstring_only() {
        assert_eq!(lines("\"\"\"top\"\"\"\nx = 1  # c\n"), vec![1, 2]);
        assert_eq!(lines("# c\n'''top\nmore'''\n"), vec![1, 2]);
        assert_eq!(
            lines("x = \"#no\"\ny = '''a\n# no\n'''\n"),
            Vec::<usize>::new()
        );
        assert_eq!(
            lines("def f():\n    \"\"\"doc\"\"\"\n    return 1\n"),
            Vec::<usize>::new()
        );
        assert_eq!(
            lines("import os\n\"\"\"not top\"\"\"\n"),
            Vec::<usize>::new()
        );
        assert_eq!(lines("s = 'it\\'s # no'  # yes\n"), vec![1]);
        assert_eq!(lines("x = r\"\\\\\"  # yes\n"), vec![1]);
    }
}
