use crate::comments::{classify, python_marks, Kind};
use crate::{checks, git};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const HELPER: &str = include_str!("clean.py");

#[derive(Default, PartialEq, Debug)]
pub struct Cleaned {
    pub text: String,
    pub comments: usize,
    pub suppressions: usize,
}

pub fn clean_comments(src: &str) -> Cleaned {
    let mut lines: Vec<Option<String>> = src.split('\n').map(|l| Some(l.to_string())).collect();
    let mut out = Cleaned::default();
    for (n, text) in python_marks(src) {
        if !text.starts_with('#') {
            continue;
        }
        let Some(kind) = classify(&text, n) else {
            continue;
        };
        let Some(Some(line)) = lines.get_mut(n - 1) else {
            continue;
        };
        let Some(code) = line.strip_suffix(text.as_str()) else {
            continue;
        };
        let code = code.trim_end();
        lines[n - 1] = if code.is_empty() {
            None
        } else {
            let cr = if text.ends_with('\r') { "\r" } else { "" };
            Some(format!("{code}{cr}"))
        };
        match kind {
            Kind::Comment => out.comments += 1,
            Kind::Suppression => out.suppressions += 1,
        }
    }
    out.text = lines.into_iter().flatten().collect::<Vec<_>>().join("\n");
    out
}

fn python(root: &Path) -> Option<PathBuf> {
    let main_root = git::main_worktree(root);
    checks::environments(root, root, &main_root)
        .into_iter()
        .map(|v| v.join("bin").join("python"))
        .find(|p| p.is_file())
        .or_else(|| checks::find_on_path("python3", &checks::search_path()))
}

fn ask(python: &Path, mode: &str, items: &[Value]) -> Result<Vec<Value>, String> {
    let mut child = Command::new(python)
        .args(["-I", "-c", HELPER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{} 실행 실패: {e}", python.display()))?;
    let request = json!({ "mode": mode, "items": items }).to_string();
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(request.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
}

fn python_files(root: &Path, paths: &[String]) -> Vec<PathBuf> {
    let mut args = vec!["ls-files", "-z", "--"];
    args.extend(paths.iter().map(String::as_str));
    git::git(root, &args)
        .split('\0')
        .filter(|n| n.ends_with(".py"))
        .map(|n| root.join(n))
        .filter(|p| p.is_file())
        .collect()
}

fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

fn setup() -> Result<(PathBuf, PathBuf), ExitCode> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let Some(root) = git::toplevel(&cwd) else {
        eprintln!("git 저장소 안에서 실행하세요");
        return Err(ExitCode::from(2));
    };
    let Some(python) = python(&root) else {
        eprintln!("python3를 찾지 못했습니다. 프로젝트 .venv를 만들거나 PATH에 python3를 두세요");
        return Err(ExitCode::from(2));
    };
    Ok((root, python))
}

fn clean(paths: &[String]) -> ExitCode {
    let (root, python) = match setup() {
        Ok(found) => found,
        Err(code) => return code,
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    let paths: Vec<String> = if paths.is_empty() {
        vec![cwd.to_string_lossy().into_owned()]
    } else {
        paths
            .iter()
            .map(|p| cwd.join(p).to_string_lossy().into_owned())
            .collect()
    };
    let files = python_files(&root, &paths);
    let dirty: Vec<PathBuf> = git::dirty_files(&root);
    let busy: Vec<&PathBuf> = files.iter().filter(|f| dirty.contains(f)).collect();
    if !busy.is_empty() {
        eprintln!("커밋하지 않은 변경이 있는 파일이 있습니다. 먼저 커밋해 주석 삭제를 따로 된 커밋으로 남기세요:");
        for f in busy.iter().take(20) {
            eprintln!("  {}", relative(&root, f));
        }
        return ExitCode::from(2);
    }
    let mut work = Vec::new();
    let mut items = Vec::new();
    for file in files {
        let Ok(before) = fs::read_to_string(&file) else {
            continue;
        };
        let cleaned = clean_comments(&before);
        items.push(json!({ "before": before, "after": cleaned.text }));
        work.push((file, before, cleaned));
    }
    let answers = match ask(&python, "clean", &items) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("검증 실패: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (mut changed, mut comments, mut suppressions, mut docstrings) = (0, 0, 0, 0);
    let mut skipped = Vec::new();
    let mut kept = Vec::new();
    for ((file, before, cleaned), answer) in work.iter().zip(&answers) {
        let name = relative(&root, file);
        if answer["ok"] != true {
            skipped.push(format!(
                "{name}: {}",
                answer["error"].as_str().unwrap_or("")
            ));
            continue;
        }
        let docstring = answer["docstring"] == "removed";
        if let Some(why) = answer["docstring"].as_str().filter(|_| !docstring) {
            kept.push(format!("{name}: {why}"));
        }
        let text = answer["text"].as_str().unwrap_or_default();
        if text == before {
            continue;
        }
        if fs::write(file, text).is_err() {
            skipped.push(format!("{name}: 쓰기 실패"));
            continue;
        }
        let mut parts = Vec::new();
        if cleaned.comments > 0 {
            parts.push(format!("주석 {}", cleaned.comments));
        }
        if cleaned.suppressions > 0 {
            parts.push(format!("검사 끄기 표시 {}", cleaned.suppressions));
        }
        if docstring {
            parts.push("파일 맨 위 설명문".to_string());
        }
        println!("{name}: {}", parts.join(", "));
        changed += 1;
        comments += cleaned.comments;
        suppressions += cleaned.suppressions;
        docstrings += usize::from(docstring);
    }
    println!(
        "\n지움: 파일 {changed}개, 주석 {comments}, 검사 끄기 표시 {suppressions}, 파일 맨 위 설명문 {docstrings}"
    );
    println!("모든 파일은 지우기 전과 코드(AST)가 같은 것을 확인했습니다");
    if !kept.is_empty() {
        println!("\n설명문을 남긴 파일 (지우면 동작이나 테스트가 바뀜):");
        for k in &kept {
            println!("  {k}");
        }
    }
    if !skipped.is_empty() {
        println!("\n건너뜀 (그대로 둠):");
        for s in &skipped {
            println!("  {s}");
        }
    }
    if changed > 0 {
        println!("\n다음:");
        if suppressions > 0 {
            println!("  - 검사 끄기 표시를 지웠으므로 ruff, ty가 실패할 수 있습니다. 원인을 고치거나 설정(per-file-ignores 등)으로 옮깁니다");
        }
        println!("  - 주석 삭제만 따로 커밋합니다. 나중에 kiln clean --verify --base <그 커밋>^로 다시 확인할 수 있습니다");
        println!("  - 다시 생기지 않도록 [tool.kiln] comments = false를 켭니다");
    }
    ExitCode::SUCCESS
}

fn verify(args: &[String]) -> ExitCode {
    let (root, python) = match setup() {
        Ok(found) => found,
        Err(code) => return code,
    };
    let reference = match args {
        [] => "HEAD".to_string(),
        [flag, r] if flag == "--base" => r.clone(),
        _ => {
            eprintln!("사용: kiln clean --verify [--base <ref>]");
            return ExitCode::from(2);
        }
    };
    let status = git::git(
        &root,
        &["diff", "--name-status", "-z", "--no-renames", &reference],
    );
    let fields: Vec<&str> = status.split('\0').filter(|s| !s.is_empty()).collect();
    let mut problems = Vec::new();
    let mut names = Vec::new();
    let mut items = Vec::new();
    for pair in fields.chunks(2) {
        let [state, name] = pair else { continue };
        if !name.ends_with(".py") {
            continue;
        }
        if *state != "M" {
            problems.push(format!("{name}: 파일이 추가되거나 지워짐"));
            continue;
        }
        let before = git::git(&root, &["show", &format!("{reference}:{name}")]);
        let after = fs::read_to_string(root.join(name)).unwrap_or_default();
        names.push(name.to_string());
        items.push(json!({ "before": before, "after": after }));
    }
    let answers = match ask(&python, "same", &items) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("검증 실패: {e}");
            return ExitCode::FAILURE;
        }
    };
    for (name, answer) in names.iter().zip(&answers) {
        if answer["ok"] != true {
            let why = answer["error"].as_str().unwrap_or("코드(AST)가 달라짐");
            problems.push(format!("{name}: {why}"));
        }
    }
    if problems.is_empty() {
        println!(
            "{reference} 이후 바뀐 파이썬 파일 {}개 모두 주석과 설명문만 달라졌습니다",
            names.len()
        );
        return ExitCode::SUCCESS;
    }
    eprintln!("주석 말고 코드도 바뀐 파일:");
    for p in &problems {
        eprintln!("  {p}");
    }
    ExitCode::FAILURE
}

pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("--verify") => verify(&args[1..]),
        _ if args.iter().any(|a| a.starts_with('-')) => {
            eprintln!("사용:\n  kiln clean [경로...]\n  kiln clean --verify [--base <ref>]");
            ExitCode::from(2)
        }
        _ => clean(args),
    }
}

#[cfg(test)]
mod tests {
    use super::clean_comments;

    #[test]
    fn removes_comments_and_suppressions_but_keeps_directives() {
        let src = "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\n# why\nimport os  # noqa: F401\nx = \"# not\"  # tail\ny = 1  # type: int\n\n\ndef f():\n    # inside\n    return 1  # pragma: no cover\n";
        let out = clean_comments(src);
        assert_eq!(
            out.text,
            "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\nimport os\nx = \"# not\"\ny = 1  # type: int\n\n\ndef f():\n    return 1  # pragma: no cover\n"
        );
        assert_eq!((out.comments, out.suppressions), (3, 1));
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let out = clean_comments("x = 1  # c\r\n# d\r\ny = 2\r\n");
        assert_eq!(out.text, "x = 1\r\ny = 2\r\n");
    }
}
