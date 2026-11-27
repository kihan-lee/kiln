use crate::comments::{c_comment_marks, classify, marks_in, CMark, Kind, C_EXTS};
use crate::{checks, git};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::str::FromStr;
use yaml_rust2::YamlLoader;

const HELPER: &str = include_str!("clean.py");
const USAGE: &str = "사용:\n  kiln clean [경로...]\n  kiln clean --verify [--base <ref>]";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    Python,
    Rust,
    C,
    Yaml,
    Shell,
    Sql,
}

pub fn lang(path: &Path) -> Option<Lang> {
    match path.extension()?.to_str()? {
        "py" => Some(Lang::Python),
        "rs" => Some(Lang::Rust),
        "yaml" | "yml" => Some(Lang::Yaml),
        "sh" | "bash" => Some(Lang::Shell),
        "sql" => Some(Lang::Sql),
        x if C_EXTS.contains(&x) => Some(Lang::C),
        _ => None,
    }
}

fn is_comment_syntax(lang: Lang, text: &str) -> bool {
    match lang {
        Lang::Python | Lang::Yaml | Lang::Shell => text.starts_with('#'),
        Lang::Sql => text.starts_with("--"),
        Lang::Rust | Lang::C => !text.starts_with("#[") && !text.starts_with("#!["),
    }
}

#[derive(Default, PartialEq, Debug)]
pub struct Cleaned {
    pub text: String,
    pub comments: usize,
    pub suppressions: usize,
    pub in_code: Vec<String>,
    pub problem: Option<String>,
}

fn split_cr(line: &str) -> (&str, &str) {
    match line.strip_suffix('\r') {
        Some(body) => (body, "\r"),
        None => (line, ""),
    }
}

fn count(out: &mut Cleaned, kind: Kind) {
    match kind {
        Kind::Comment => out.comments += 1,
        Kind::Suppression => out.suppressions += 1,
    }
}

fn clean_c(src: &str) -> Cleaned {
    let mut lines: Vec<Option<String>> = src.split('\n').map(|l| Some(l.to_string())).collect();
    let mut out = Cleaned::default();
    for mark in c_comment_marks(src) {
        let (pieces, start, end) = match mark {
            CMark::Pragma(n, text) => {
                if classify(&text, n) == Some(Kind::Suppression) {
                    out.in_code.push(format!("{n}: {text}"));
                }
                continue;
            }
            CMark::Comment { pieces, start, end } => (pieces, start, end),
        };
        let (Some((first_line, first)), Some((last_line, last))) = (pieces.first(), pieces.last())
        else {
            continue;
        };
        let Some(kind) = classify(first, *first_line) else {
            continue;
        };
        let line = |n: usize| lines.get(n - 1).cloned().flatten();
        let (Some(first_full), Some(last_full)) = (line(start), line(end)) else {
            continue;
        };
        let untouched = (start..=end).all(|n| line(n).is_some());
        let closes_alone =
            end == start || (*last_line == end && split_cr(&last_full).0.trim() == last);
        let (body, cr) = split_cr(&first_full);
        let code = body.trim_end().strip_suffix(first.as_str());
        let (Some(code), true, true, true) = (code, untouched, closes_alone, *first_line == start)
        else {
            continue;
        };
        let code = code.trim_end();
        let kept = (!code.is_empty()).then(|| format!("{code}{cr}"));
        for n in start..=end {
            lines[n - 1] = None;
        }
        lines[start - 1] = kept;
        count(&mut out, kind);
    }
    out.text = lines.into_iter().flatten().collect::<Vec<_>>().join("\n");
    out
}

fn token_ranges(stream: proc_macro2::TokenStream, starts: &[usize], out: &mut Vec<(usize, usize)>) {
    let offset = |at: proc_macro2::LineColumn| starts.get(at.line - 1).map_or(0, |s| s + at.column);
    let range = |span: proc_macro2::Span| (offset(span.start()), offset(span.end()));
    for tree in stream {
        match tree {
            proc_macro2::TokenTree::Group(group) => {
                out.push(range(group.span_open()));
                out.push(range(group.span_close()));
                token_ranges(group.stream(), starts, out);
            }
            other => out.push(range(other.span())),
        }
    }
}

fn rust_comment_ranges(chars: &[char]) -> Result<Vec<(usize, usize)>, String> {
    let src: String = chars.iter().collect();
    let stream =
        proc_macro2::TokenStream::from_str(&src).map_err(|e| format!("구문 분석 실패: {e}"))?;
    let mut starts = vec![0];
    starts.extend(
        chars
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == '\n')
            .map(|(i, _)| i + 1),
    );
    let mut tokens = Vec::new();
    token_ranges(stream, &starts, &mut tokens);
    tokens.sort_unstable();
    let mut comments = Vec::new();
    let mut at = 0;
    for (from, to) in tokens
        .into_iter()
        .chain(std::iter::once((chars.len(), chars.len())))
    {
        let mut i = at;
        while i < from {
            if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
                let end = (i..from).find(|&j| chars[j] == '\n').unwrap_or(from);
                comments.push((i, end));
                i = end;
            } else if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                let (mut depth, mut j) = (0, i);
                while j < from {
                    if chars[j] == '/' && chars.get(j + 1) == Some(&'*') {
                        depth += 1;
                        j += 2;
                    } else if chars[j] == '*' && chars.get(j + 1) == Some(&'/') {
                        depth -= 1;
                        j += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        j += 1;
                    }
                }
                comments.push((i, j));
                i = j;
            } else {
                i += 1;
            }
        }
        at = at.max(to);
    }
    Ok(comments)
}

fn clean_rust(src: &str) -> Cleaned {
    let mut out = Cleaned::default();
    for (n, text) in marks_in(src, "rs") {
        if !is_comment_syntax(Lang::Rust, &text) && classify(&text, n) == Some(Kind::Suppression) {
            out.in_code.push(format!("{n}: {text}"));
        }
    }
    let chars: Vec<char> = src.chars().collect();
    let ranges = match rust_comment_ranges(&chars) {
        Ok(ranges) => ranges,
        Err(why) => {
            out.text = src.to_string();
            out.problem = Some(why);
            return out;
        }
    };
    let mut removed = vec![false; chars.len()];
    let mut touched = std::collections::HashSet::new();
    let mut lines_at = Vec::with_capacity(chars.len() + 1);
    let mut current = 1;
    for c in &chars {
        lines_at.push(current);
        current += usize::from(*c == '\n');
    }
    lines_at.push(current);
    let line_of = |i: usize| lines_at[i];
    for (from, to) in ranges {
        let text: String = chars[from..to].iter().collect();
        let line = line_of(from);
        let first = text.lines().next().unwrap_or("").trim().to_string();
        let Some(kind) = classify(&first, line) else {
            continue;
        };
        (from..to).for_each(|i| removed[i] = chars[i] != '\n');
        (line..=line_of(to)).for_each(|n| {
            touched.insert(n);
        });
        count(&mut out, kind);
    }
    let kept: String = chars
        .iter()
        .zip(&removed)
        .filter(|(_, r)| !**r)
        .map(|(c, _)| c)
        .collect();
    out.text = kept
        .split('\n')
        .enumerate()
        .filter_map(|(i, line)| {
            if !touched.contains(&(i + 1)) {
                return Some(line.to_string());
            }
            let (body, cr) = split_cr(line);
            let body = body.trim_end();
            (!body.is_empty()).then(|| format!("{body}{cr}"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    out
}

pub fn clean_comments(src: &str, lang: Lang, ext: &str) -> Cleaned {
    match lang {
        Lang::C => return clean_c(src),
        Lang::Rust => return clean_rust(src),
        _ => {}
    }
    let mut lines: Vec<Option<String>> = src.split('\n').map(|l| Some(l.to_string())).collect();
    let mut out = Cleaned::default();
    for (n, text) in marks_in(src, ext) {
        let Some(kind) = classify(&text, n) else {
            continue;
        };
        if !is_comment_syntax(lang, &text) {
            if kind == Kind::Suppression {
                out.in_code.push(format!("{n}: {}", text.trim()));
            }
            continue;
        }
        let Some(Some(line)) = lines.get(n - 1) else {
            continue;
        };
        let (body, cr) = split_cr(line);
        let mark = text.trim();
        let replacement = if body.trim() == mark {
            None
        } else if let Some(code) = body.trim_end().strip_suffix(mark) {
            Some(format!("{}{cr}", code.trim_end()))
        } else {
            continue;
        };
        lines[n - 1] = replacement;
        count(&mut out, kind);
    }
    out.text = lines.into_iter().flatten().collect::<Vec<_>>().join("\n");
    out
}

fn tool(name: &str) -> Option<PathBuf> {
    checks::find_on_path(name, &checks::search_path())
}

fn python(root: &Path) -> Option<PathBuf> {
    let main_root = git::main_worktree(root);
    checks::environments(root, root, &main_root)
        .into_iter()
        .map(|v| v.join("bin").join("python"))
        .find(|p| p.is_file())
        .or_else(|| tool("python3"))
}

fn piped(cmd: &mut Command, input: &str) -> Result<(bool, String, String), String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("stdin 없음")?;
    let input = input.to_string();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = writer.join();
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

fn ask_python(python: &Path, mode: &str, items: &[Value]) -> Result<Vec<Value>, String> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let request = json!({ "mode": mode, "items": items }).to_string();
    let (ok, out, err) = piped(Command::new(python).args(["-I", "-c", HELPER]), &request)?;
    if !ok {
        return Err(err);
    }
    serde_json::from_str(&out).map_err(|e| e.to_string())
}

fn rust_tokens(src: &str) -> Result<String, String> {
    proc_macro2::TokenStream::from_str(src)
        .map(|t| t.to_string())
        .map_err(|e| format!("구문 분석 실패: {e}"))
}

fn c_tokens(src: &str) -> Result<Vec<String>, String> {
    let clang = tool("clang").ok_or("clang이 없어 확인할 수 없음")?;
    let args = [
        "-x",
        "c++",
        "-std=c++17",
        "-fsyntax-only",
        "-Xclang",
        "-dump-raw-tokens",
        "-",
    ];
    let (_, _, err) = piped(Command::new(clang).args(args), src)?;
    let mut tokens = Vec::new();
    let mut rest = err.as_str();
    while let Some(at) = rest.find("Loc=<") {
        let record = rest[..at].trim_start_matches('\n');
        let head = record.split('\t').next().unwrap_or("");
        let (kind, spelling) = head.split_once(' ').unwrap_or((head, ""));
        let blank = spelling.trim_matches('\'').trim().is_empty();
        if kind != "comment" && !(kind == "unknown" && blank) {
            tokens.push(head.to_string());
        }
        rest = rest[at..]
            .find(">\n")
            .map_or("", |end| &rest[at + end + 2..]);
    }
    Ok(tokens)
}

fn yaml_docs(src: &str) -> Result<Vec<yaml_rust2::Yaml>, String> {
    YamlLoader::load_from_str(src).map_err(|e| format!("구문 분석 실패: {e}"))
}

fn without_positions(value: &mut Value) {
    let is_pos = |v: &Value| {
        v.as_object()
            .is_some_and(|o| o.contains_key("Offset") && o.contains_key("Line"))
    };
    let is_comments = |v: &Value| {
        v.as_array().is_some_and(|a| {
            a.iter()
                .all(|c| c.as_object().is_some_and(|o| o.contains_key("Hash")))
        })
    };
    match value {
        Value::Object(map) => {
            map.retain(|_, v| !is_pos(v) && !is_comments(v));
            map.values_mut().for_each(without_positions);
        }
        Value::Array(items) => items.iter_mut().for_each(without_positions),
        _ => {}
    }
}

fn shell_tree(name: &str, src: &str) -> Result<Value, String> {
    let shfmt = tool("shfmt").ok_or("shfmt가 없어 확인할 수 없음 (brew install shfmt)")?;
    let (ok, out, err) = piped(
        Command::new(shfmt).args(["--to-json", "--filename", name]),
        src,
    )?;
    if !ok {
        return Err(format!(
            "구문 분석 실패: {}",
            err.lines().next().unwrap_or("")
        ));
    }
    let mut tree: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    without_positions(&mut tree);
    Ok(tree)
}

fn same_code(lang: Lang, name: &str, before: &str, after: &str) -> Result<bool, String> {
    match lang {
        Lang::Rust => Ok(rust_tokens(before)? == rust_tokens(after)?),
        Lang::C => Ok(c_tokens(before)? == c_tokens(after)?),
        Lang::Yaml => Ok(yaml_docs(before)? == yaml_docs(after)?),
        Lang::Shell => Ok(shell_tree(name, before)? == shell_tree(name, after)?),
        Lang::Sql => Err("SQL은 코드가 같은지 확인할 도구가 없어 건너뜀".into()),
        Lang::Python => Err("파이썬은 따로 확인".into()),
    }
}

fn chart_of(root: &Path, file: &Path) -> Option<PathBuf> {
    file.ancestors()
        .skip(1)
        .take_while(|d| d.starts_with(root))
        .find(|d| d.join("Chart.yaml").is_file())
        .map(Path::to_path_buf)
}

fn render(chart: &Path) -> Result<Vec<yaml_rust2::Yaml>, String> {
    let helm = tool("helm").ok_or("helm이 없어 템플릿을 확인할 수 없음")?;
    let output = Command::new(helm)
        .args(["template", "kiln"])
        .arg(chart)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "helm template 실패: {}",
            err.lines().next().unwrap_or("")
        ));
    }
    yaml_docs(&String::from_utf8_lossy(&output.stdout))
}

fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

fn git_root() -> Result<PathBuf, ExitCode> {
    let cwd = std::env::current_dir().unwrap_or_default();
    git::toplevel(&cwd).ok_or_else(|| {
        eprintln!("git 저장소 안에서 실행하세요");
        ExitCode::from(2)
    })
}

fn target_files(root: &Path, paths: &[String]) -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let paths: Vec<String> = if paths.is_empty() {
        vec![cwd.to_string_lossy().into_owned()]
    } else {
        paths
            .iter()
            .map(|p| cwd.join(p).to_string_lossy().into_owned())
            .collect()
    };
    let mut args = vec!["ls-files", "-z", "--"];
    args.extend(paths.iter().map(String::as_str));
    git::git(root, &args)
        .split('\0')
        .filter(|n| !n.is_empty())
        .map(|n| root.join(n))
        .filter(|p| p.is_file() && lang(p).is_some())
        .collect()
}

struct Job {
    file: PathBuf,
    before: String,
    cleaned: Cleaned,
}

#[derive(Default)]
struct Summary {
    files: usize,
    comments: usize,
    suppressions: usize,
    docstrings: usize,
    kept_docstrings: Vec<String>,
    in_code: Vec<String>,
    skipped: Vec<String>,
}

impl Summary {
    fn done(&mut self, root: &Path, job: &Job, text: &str, docstring: bool) {
        let name = relative(root, &job.file);
        if fs::write(&job.file, text).is_err() {
            self.skipped.push(format!("{name}: 쓰기 실패"));
            return;
        }
        let mut parts = Vec::new();
        if job.cleaned.comments > 0 {
            parts.push(format!("주석 {}", job.cleaned.comments));
        }
        if job.cleaned.suppressions > 0 {
            parts.push(format!("검사 끄기 표시 {}", job.cleaned.suppressions));
        }
        if docstring {
            parts.push("파일 맨 위 설명문".to_string());
        }
        println!("{name}: {}", parts.join(", "));
        self.files += 1;
        self.comments += job.cleaned.comments;
        self.suppressions += job.cleaned.suppressions;
        self.docstrings += usize::from(docstring);
    }

    fn skip(&mut self, root: &Path, job: &Job, why: &str) {
        self.skipped
            .push(format!("{}: {why}", relative(root, &job.file)));
    }

    fn print(&self) {
        println!(
            "\n지움: 파일 {}개, 주석 {}, 검사 끄기 표시 {}, 파일 맨 위 설명문 {}",
            self.files, self.comments, self.suppressions, self.docstrings
        );
        println!("지운 파일은 모두 지우기 전과 코드가 같은 것을 확인했습니다 (파이썬 AST, Rust와 C 토큰, YAML 데이터와 helm 렌더 결과, shfmt 구문 트리)");
        let sections = [
            (
                "설명문을 남긴 파일 (지우면 동작이나 테스트가 바뀜)",
                &self.kept_docstrings,
            ),
            (
                "코드에 있는 검사 끄기 표시 (주석이 아니라 직접 정리)",
                &self.in_code,
            ),
            ("건너뜀 (그대로 둠)", &self.skipped),
        ];
        for (title, items) in sections {
            if !items.is_empty() {
                println!("\n{title}:");
                for item in items {
                    println!("  {item}");
                }
            }
        }
        if self.files > 0 {
            println!("\n다음:");
            if self.suppressions > 0 {
                println!("  - 검사 끄기 표시를 지웠으므로 린트가 실패할 수 있습니다. 원인을 고치거나 설정으로 옮깁니다");
            }
            println!("  - 주석 삭제만 따로 커밋합니다. 나중에 kiln clean --verify --base <그 커밋>^로 다시 확인할 수 있습니다");
            println!("  - 다시 생기지 않도록 프로젝트 설정에 comments = false를 켭니다");
        }
    }
}

fn clean(paths: &[String]) -> ExitCode {
    let root = match git_root() {
        Ok(root) => root,
        Err(code) => return code,
    };
    let files = target_files(&root, paths);
    let dirty = git::dirty_files(&root);
    let busy: Vec<&PathBuf> = files.iter().filter(|f| dirty.contains(f)).collect();
    if !busy.is_empty() {
        eprintln!("커밋하지 않은 변경이 있는 파일이 있습니다. 먼저 커밋해 주석 삭제를 따로 된 커밋으로 남기세요:");
        for f in busy.iter().take(20) {
            eprintln!("  {}", relative(&root, f));
        }
        return ExitCode::from(2);
    }
    let mut summary = Summary::default();
    let mut python_jobs = Vec::new();
    let mut charts: BTreeMap<PathBuf, Vec<Job>> = BTreeMap::new();
    for file in files {
        let Ok(before) = fs::read_to_string(&file) else {
            continue;
        };
        let lang = lang(&file).unwrap_or(Lang::Sql);
        let ext = file.extension().and_then(|x| x.to_str()).unwrap_or("");
        let cleaned = clean_comments(&before, lang, ext);
        let name = relative(&root, &file);
        summary
            .in_code
            .extend(cleaned.in_code.iter().map(|m| format!("{name}:{m}")));
        let job = Job {
            file,
            before,
            cleaned,
        };
        if lang == Lang::Python {
            python_jobs.push(job);
            continue;
        }
        if let Some(why) = &job.cleaned.problem {
            summary.skip(&root, &job, why);
            continue;
        }
        if job.cleaned.text == job.before {
            continue;
        }
        match same_code(lang, &name, &job.before, &job.cleaned.text) {
            Ok(true) => {
                let text = job.cleaned.text.clone();
                summary.done(&root, &job, &text, false);
            }
            Ok(false) => summary.skip(&root, &job, "지운 뒤 코드가 달라짐"),
            Err(why) => match chart_of(&root, &job.file).filter(|_| lang == Lang::Yaml) {
                Some(chart) => charts.entry(chart).or_default().push(job),
                None => summary.skip(&root, &job, &why),
            },
        }
    }
    clean_python(&root, python_jobs, &mut summary);
    for (chart, jobs) in charts {
        clean_chart(&root, &chart, &jobs, &mut summary);
    }
    summary.print();
    ExitCode::SUCCESS
}

fn clean_python(root: &Path, jobs: Vec<Job>, summary: &mut Summary) {
    if jobs.is_empty() {
        return;
    }
    let Some(python) = python(root) else {
        for job in &jobs {
            summary.skip(root, job, "python3가 없어 확인할 수 없음");
        }
        return;
    };
    let items: Vec<Value> = jobs
        .iter()
        .map(|j| json!({ "before": j.before, "after": j.cleaned.text }))
        .collect();
    let answers = match ask_python(&python, "clean", &items) {
        Ok(a) => a,
        Err(e) => {
            for job in &jobs {
                summary.skip(root, job, &format!("파이썬 확인 실패: {e}"));
            }
            return;
        }
    };
    for (job, answer) in jobs.iter().zip(&answers) {
        if answer["ok"] != true {
            summary.skip(root, job, answer["error"].as_str().unwrap_or(""));
            continue;
        }
        let docstring = answer["docstring"] == "removed";
        if let Some(why) = answer["docstring"].as_str().filter(|_| !docstring) {
            let name = relative(root, &job.file);
            summary.kept_docstrings.push(format!("{name}: {why}"));
        }
        let text = answer["text"].as_str().unwrap_or_default();
        if text != job.before {
            summary.done(root, job, text, docstring);
        }
    }
}

fn clean_chart(root: &Path, chart: &Path, jobs: &[Job], summary: &mut Summary) {
    let before = match render(chart) {
        Ok(docs) => docs,
        Err(why) => {
            jobs.iter().for_each(|j| summary.skip(root, j, &why));
            return;
        }
    };
    for job in jobs {
        let _ = fs::write(&job.file, &job.cleaned.text);
    }
    let after = render(chart);
    for job in jobs {
        let _ = fs::write(&job.file, &job.before);
    }
    match after {
        Ok(docs) if docs == before => {
            for job in jobs {
                let text = job.cleaned.text.clone();
                summary.done(root, job, &text, false);
            }
        }
        _ => jobs
            .iter()
            .for_each(|j| summary.skip(root, j, "지운 뒤 helm 렌더 결과가 달라짐")),
    }
}

fn chart_at(root: &Path, reference: &str, chart: &Path) -> Result<Vec<yaml_rust2::Yaml>, String> {
    let tmp = std::env::temp_dir().join(format!("kiln-clean-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let rel = relative(root, chart);
    let archive = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["archive", reference, "--", &rel])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let ok = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(&tmp)
        .stdin(archive.stdout.ok_or("git archive 실패")?)
        .status()
        .map_err(|e| e.to_string())?
        .success();
    let result = if ok {
        render(&tmp.join(&rel))
    } else {
        Err("git archive 실패".into())
    };
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn verify(args: &[String]) -> ExitCode {
    let root = match git_root() {
        Ok(root) => root,
        Err(code) => return code,
    };
    let reference = match args {
        [] => "HEAD".to_string(),
        [flag, r] if flag == "--base" => r.clone(),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let status = git::git(
        &root,
        &["diff", "--name-status", "-z", "--no-renames", &reference],
    );
    let fields: Vec<&str> = status.split('\0').filter(|s| !s.is_empty()).collect();
    let (mut problems, mut unknown) = (Vec::new(), Vec::new());
    let (mut names, mut items) = (Vec::new(), Vec::new());
    let mut charts: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    let mut checked = 0;
    for pair in fields.chunks(2) {
        let [state, name] = pair else { continue };
        let path = root.join(name);
        let Some(lang) = lang(&path) else { continue };
        if *state != "M" {
            problems.push(format!("{name}: 파일이 추가되거나 지워짐"));
            continue;
        }
        let before = git::git(&root, &["show", &format!("{reference}:{name}")]);
        let after = fs::read_to_string(&path).unwrap_or_default();
        if lang == Lang::Python {
            names.push(name.to_string());
            items.push(json!({ "before": before, "after": after }));
            continue;
        }
        match same_code(lang, name, &before, &after) {
            Ok(true) => checked += 1,
            Ok(false) => problems.push(format!("{name}: 코드가 달라짐")),
            Err(why) => match chart_of(&root, &path).filter(|_| lang == Lang::Yaml) {
                Some(chart) => charts.entry(chart).or_default().push(name.to_string()),
                None => unknown.push(format!("{name}: {why}")),
            },
        }
    }
    if !items.is_empty() {
        match python(&root).ok_or("python3가 없어 확인할 수 없음".to_string()) {
            Ok(py) => match ask_python(&py, "same", &items) {
                Ok(answers) => {
                    for (name, answer) in names.iter().zip(&answers) {
                        match (answer["ok"] == true, answer["error"].as_str()) {
                            (true, _) => checked += 1,
                            (false, Some(why)) => unknown.push(format!("{name}: {why}")),
                            (false, None) => problems.push(format!("{name}: 코드(AST)가 달라짐")),
                        }
                    }
                }
                Err(e) => unknown.extend(names.iter().map(|n| format!("{n}: {e}"))),
            },
            Err(e) => unknown.extend(names.iter().map(|n| format!("{n}: {e}"))),
        }
    }
    for (chart, files) in charts {
        match (chart_at(&root, &reference, &chart), render(&chart)) {
            (Ok(a), Ok(b)) if a == b => checked += files.len(),
            (Ok(_), Ok(_)) => problems.extend(
                files
                    .iter()
                    .map(|f| format!("{f}: helm 렌더 결과가 달라짐")),
            ),
            (Err(e), _) | (_, Err(e)) => unknown.extend(files.iter().map(|f| format!("{f}: {e}"))),
        }
    }
    if !unknown.is_empty() {
        eprintln!("확인하지 못한 파일:");
        unknown.iter().for_each(|u| eprintln!("  {u}"));
    }
    if !problems.is_empty() {
        eprintln!("주석 말고 코드도 바뀐 파일:");
        problems.iter().for_each(|p| eprintln!("  {p}"));
    }
    if problems.is_empty() && unknown.is_empty() {
        println!("{reference} 이후 바뀐 파일 {checked}개 모두 주석과 설명문만 달라졌습니다");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("--verify") => verify(&args[1..]),
        _ if args.iter().any(|a| a.starts_with('-')) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        _ => clean(args),
    }
}

#[cfg(test)]
mod tests {
    use super::{clean_c, clean_comments, same_code, Lang};

    #[test]
    fn removes_python_comments_and_suppressions_but_keeps_directives() {
        let src = "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\n# why\nimport os  # noqa: F401\nx = \"# not\"  # tail\ny = 1  # type: int\n\n\ndef f():\n    # inside\n    return 1  # pragma: no cover\n";
        let out = clean_comments(src, Lang::Python, "py");
        assert_eq!(
            out.text,
            "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\nimport os\nx = \"# not\"\ny = 1  # type: int\n\n\ndef f():\n    return 1  # pragma: no cover\n"
        );
        assert_eq!((out.comments, out.suppressions), (3, 1));
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let out = clean_comments("x = 1  # c\r\n# d\r\ny = 2\r\n", Lang::Python, "py");
        assert_eq!(out.text, "x = 1\r\ny = 2\r\n");
    }

    #[test]
    fn removes_rust_comments_but_leaves_allow_attributes_for_review() {
        let src = "/// doc\n// why\nfn f() {}\n#[allow(dead_code)]\nfn g() {}\n";
        let out = clean_comments(src, Lang::Rust, "rs");
        assert_eq!(
            out.text,
            "/// doc\nfn f() {}\n#[allow(dead_code)]\nfn g() {}\n"
        );
        assert_eq!(out.in_code, vec!["4: #[allow(dead_code)]".to_string()]);
        assert_eq!(same_code(Lang::Rust, "a.rs", src, &out.text), Ok(true));
        assert_eq!(
            same_code(Lang::Rust, "a.rs", src, "fn f() {}\nfn h() {}\n"),
            Ok(false)
        );
    }

    #[test]
    fn rust_comments_are_found_between_tokens() {
        let src = "fn f() -> &'static str {\n    let s = \"// not\"; // tail\n    let r = r#\"/* no */\"#;\n    g(/* mid */ 1);\n    /* outer /* nested */ still */\n    s\n}\n";
        let out = clean_comments(src, Lang::Rust, "rs");
        assert_eq!(
            out.text,
            "fn f() -> &'static str {\n    let s = \"// not\";\n    let r = r#\"/* no */\"#;\n    g( 1);\n    s\n}\n"
        );
        assert_eq!(out.comments, 3);
        assert_eq!(same_code(Lang::Rust, "a.rs", src, &out.text), Ok(true));
    }

    #[test]
    fn removes_c_comments_including_multiline_blocks() {
        let src =
            "// why\nint x = 1; // one\n/* block\n   more */\n/** doc */\nint y;\n#pragma unroll\n";
        let out = clean_comments(src, Lang::C, "cu");
        assert_eq!(out.text, "int x = 1;\n/** doc */\nint y;\n#pragma unroll\n");
        assert_eq!(out.comments, 3);
    }

    #[test]
    fn c_block_comments_are_removed_whole_or_not_at_all() {
        let src = "/* example:\n#if X\ncoding and more\n*/\nint a;\nint b; /* start\n end */\n/* tail\n */ int c;\n#  pragma nv_diag_suppress 177\n// cont \\\nstill\n";
        let out = clean_c(src);
        assert_eq!(
            out.text,
            "int a;\nint b;\n/* tail\n */ int c;\n#  pragma nv_diag_suppress 177\n// cont \\\nstill\n"
        );
        assert_eq!(out.comments, 2);
        assert_eq!(
            out.in_code,
            vec!["10: #pragma nv_diag_suppress 177".to_string()]
        );
    }

    #[test]
    fn removes_yaml_comments_but_keeps_block_scalars_and_annotations() {
        let src = "# top\na: 1  # inline\nscript: |\n  # kept\n  echo\nmetadata:\n  annotations:\n    ignore-check.kube-linter.io/x: \"why\"\n";
        let out = clean_comments(src, Lang::Yaml, "yaml");
        assert_eq!(
            out.text,
            "a: 1\nscript: |\n  # kept\n  echo\nmetadata:\n  annotations:\n    ignore-check.kube-linter.io/x: \"why\"\n"
        );
        assert_eq!(out.in_code.len(), 1);
        assert_eq!(same_code(Lang::Yaml, "a.yaml", src, &out.text), Ok(true));
        assert_eq!(same_code(Lang::Yaml, "a.yaml", src, "a: 2\n"), Ok(false));
    }

    #[test]
    fn removes_shell_comments_outside_heredocs() {
        let src = "#!/bin/bash\n# why\necho \"a # b\"  # tail\ncat <<EOF\n# kept\nEOF\n";
        let out = clean_comments(src, Lang::Shell, "sh");
        assert_eq!(
            out.text,
            "#!/bin/bash\necho \"a # b\"\ncat <<EOF\n# kept\nEOF\n"
        );
    }
}
