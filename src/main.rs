mod checks;
mod comments;
mod commit_msg;
mod config;
mod docs;
mod git;
mod k8s;
mod scope;
mod state;
mod stats;

use checks::{Outcome, Tests};
use comments::Kind;
use config::Config;
use scope::Baseline;
use serde_json::{json, Value};
use state::State;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

const MAX_BLOCKS: u32 = 3;
const TAIL_LINES: usize = 40;
const FINDING_LIMIT: usize = 20;
const USAGE: &str = "사용:
  kiln check --staged        커밋할 내용 검사 (pre-commit)
  kiln check --base <ref>    <ref>와 갈라진 뒤 바뀐 내용 검사 (CI)
  kiln check --all           프로젝트 전체 검사 (기존 주석은 목록만 보여 줌)
  kiln commit-msg <file>     커밋 메시지 형식 검사 (commit-msg)
  kiln hook start|track|stop Claude Code 훅
  kiln status                이 위치에서 어떤 검사가 도는지 보기
  kiln stats                 훅 기록 요약";

struct Failure {
    project: PathBuf,
    check: String,
    count: usize,
    report: String,
}

#[derive(Default)]
struct Report {
    failures: Vec<Failure>,
    existing: Vec<Failure>,
    skipped: Vec<String>,
    rules: Vec<PathBuf>,
}

fn findings_failure(dir: &Path, found: &[comments::Finding], existing: bool) -> Vec<Failure> {
    let (age, limit) = if existing {
        ("기존", usize::MAX)
    } else {
        ("새로 추가된", FINDING_LIMIT)
    };
    let mut failures = Vec::new();
    for (kind, check, title, guide) in [
        (
            Kind::Suppression,
            "suppressions",
            "검사 끄기 표시",
            "검사를 끄는 표시(noqa, type: ignore, #[allow] 등)는 쓰지 않습니다. 원인을 고치세요. 정말 필요한 예외는 줄에 붙이지 말고 설정 파일(pyproject.toml의 per-file-ignores, Cargo.toml의 [lints])에 모읍니다.",
        ),
        (
            Kind::Comment,
            "comments",
            "주석과 파일 맨 위 설명문",
            "주석 대신 이름, 타입, 테스트로 뜻을 드러내세요. 파일 맨 위 설명문은 쓰지 않습니다.",
        ),
    ] {
        let hits: Vec<&str> = found
            .iter()
            .filter(|f| f.kind == kind)
            .map(|f| f.text.as_str())
            .collect();
        if hits.is_empty() {
            continue;
        }
        let shown: Vec<&str> = hits.iter().take(limit).copied().collect();
        failures.push(Failure {
            project: dir.to_path_buf(),
            check: check.into(),
            count: hits.len(),
            report: format!(
                "{}$ {age} {title} {}건\n{}\n{guide}",
                dir.display(),
                hits.len(),
                shown.join("\n")
            ),
        });
    }
    failures
}

fn evaluate(
    root: &Path,
    files: &[PathBuf],
    whole: bool,
    baseline: &Baseline,
    hook: bool,
) -> Report {
    let path = checks::search_path();
    let main_root = git::main_worktree(root);
    let config = Config::load();
    let mut report = Report::default();
    if let Some(error) = &config.error {
        report
            .skipped
            .push(format!("설정 파일 오류로 무시함 ({error})"));
    }
    for dir in checks::projects(root, files) {
        let inside: Vec<PathBuf> = files
            .iter()
            .filter(|f| f.starts_with(&dir))
            .cloned()
            .collect();
        if let Some(allowed) = docs_rule(&dir, root, &main_root, &config) {
            if hook {
                report.rules.extend(docs::rule_files(&dir, &inside));
            }
            let stray = docs::stray(root, &dir, &inside, baseline, &allowed);
            if !stray.is_empty() {
                report.failures.push(Failure {
                    project: dir.clone(),
                    check: "docs".into(),
                    count: stray.len(),
                    report: format!(
                        "{}$ 허용 경로 밖의 새 문서 파일 {}건\n{}\n작업 메모나 요약은 파일로 남기지 말고 답변에 쓰세요. 사용자가 요청한 문서라면 허용 경로({})로 옮기세요.",
                        dir.display(),
                        stray.len(),
                        stray.join("\n"),
                        allowed.join(", ")
                    ),
                });
                continue;
            }
        }
        if comment_rule(&dir, root, &main_root, &config).0 {
            let found = comments::new_findings(root, &dir, &inside, baseline);
            if whole {
                report.existing.extend(findings_failure(&dir, &found, true));
            } else if !found.is_empty() {
                report
                    .failures
                    .extend(findings_failure(&dir, &found, false));
                continue;
            }
        }
        let venvs = checks::environments(&dir, root, &main_root);
        let scope = (!whole).then_some(inside.as_slice());
        let tests = if !hook {
            Tests::Full
        } else if hook_tests(&dir, root, &main_root, &config).0 {
            Tests::FailFast
        } else {
            Tests::Skip
        };
        let plan = checks::plan(&dir, scope, &venvs, &path, tests);
        for name in plan.missing {
            report
                .skipped
                .push(format!("{name} ({}에서 찾지 못함)", dir.display()));
        }
        let mut to_run = plan.checks;
        match config::custom_checks(&dir) {
            Ok(list) => to_run.extend(checks::custom(scope, hook, &list)),
            Err(error) => report
                .skipped
                .push(format!("직접 정한 검사 설정 오류로 무시함 ({error})")),
        }
        for check in to_run {
            match checks::run(&check, &dir, &path, TAIL_LINES) {
                Outcome::Passed => {}
                Outcome::Missing => {
                    report
                        .skipped
                        .push(format!("{} ({}에서 실행 불가)", check.name, dir.display()))
                }
                Outcome::Failed(tail) => {
                    let shown = check.shown.replace(&format!("{}/", dir.display()), "");
                    report.failures.push(Failure {
                        project: dir.clone(),
                        check: check.name.clone(),
                        count: 1,
                        report: format!("{}$ {shown}\n{tail}", dir.display()),
                    });
                    break;
                }
            }
        }
    }
    report
}

fn rule(
    dir: &Path,
    root: &Path,
    main_root: &Path,
    config: &Config,
    key: &str,
) -> Option<(bool, String)> {
    let mut dirs = vec![dir.to_path_buf()];
    if let Ok(rel) = dir.strip_prefix(root) {
        dirs.push(main_root.join(rel));
    }
    if let Some(value) = config.setting(&dirs, key) {
        let source = config
            .source
            .as_ref()
            .map_or(String::new(), |p| p.display().to_string());
        return Some((value, source));
    }
    config::local(dir, key).map(|(value, source)| (value, source.into()))
}

fn docs_rule(dir: &Path, root: &Path, main_root: &Path, config: &Config) -> Option<Vec<String>> {
    let mut dirs = vec![dir.to_path_buf()];
    if let Ok(rel) = dir.strip_prefix(root) {
        dirs.push(main_root.join(rel));
    }
    config.docs(&dirs).or_else(|| config::local_docs(dir))
}

fn rule_notice(root: &Path, state: &State, rules: &[PathBuf]) -> Option<Failure> {
    if rules.is_empty() {
        return None;
    }
    let mut stamp: Vec<String> = rules
        .iter()
        .map(|f| format!("{}:{}", f.display(), state::content_hash(f)))
        .collect();
    stamp.sort();
    let fingerprint = state::hash(&stamp.iter().map(String::as_str).collect::<Vec<_>>());
    if State::read(&state.rules) == fingerprint {
        return None;
    }
    let _ = fs::write(&state.rules, &fingerprint);
    let names: Vec<String> = rules
        .iter()
        .map(|f| f.strip_prefix(root).unwrap_or(f).display().to_string())
        .collect();
    Some(Failure {
        project: root.to_path_buf(),
        check: "rules".into(),
        count: rules.len(),
        report: format!(
            "규칙 파일이 바뀌었습니다 (같은 내용으로는 다시 막지 않음)\n{}\n바꾼 내용을 린트 규칙, 타입, 테스트 같은 검사로 옮길 수 있으면 옮기고 규칙 파일에서는 빼세요. 검사로 옮길 수 없는 판단만 글로 남깁니다. 확인했으면 그대로 끝내도 됩니다.",
            names.join("\n")
        ),
    })
}

fn comment_rule(dir: &Path, root: &Path, main_root: &Path, config: &Config) -> (bool, String) {
    match rule(dir, root, main_root, config, "comments") {
        Some((allowed, source)) => (!allowed, source),
        None => (false, "설정 없음".into()),
    }
}

fn hook_tests(dir: &Path, root: &Path, main_root: &Path, config: &Config) -> (bool, String) {
    rule(dir, root, main_root, config, "tests").unwrap_or((false, "설정 없음".into()))
}

fn status() -> ExitCode {
    let cwd = env::current_dir().unwrap_or_default();
    let Some(root) = git::toplevel(&cwd) else {
        println!("git 저장소가 아니라 kiln이 아무것도 하지 않습니다");
        return ExitCode::SUCCESS;
    };
    let config = Config::load();
    println!("저장소: {}", root.display());
    match (&config.source, &config.error) {
        (_, Some(error)) => println!("설정 파일: 오류, 무시함 ({error})"),
        (Some(source), None) => println!("설정 파일: {}", source.display()),
        (None, None) => println!("설정 파일: 없음 ({})", Config::path().display()),
    }
    let Some(dir) = cwd
        .ancestors()
        .take_while(|d| d.starts_with(&root))
        .find(|d| checks::is_project(d))
    else {
        println!(
            "pyproject.toml, Cargo.toml, CMakeLists.txt, kiln.toml, .yamllint가 없어 돌릴 검사가 없습니다"
        );
        return ExitCode::SUCCESS;
    };
    let main_root = git::main_worktree(&root);
    println!("프로젝트: {}", dir.display());
    let (on, source) = comment_rule(dir, &root, &main_root, &config);
    println!(
        "  새 주석, 검사 끄기 표시: {} ({source})",
        if on { "막음" } else { "검사 안 함" }
    );
    let venvs = checks::environments(dir, &root, &main_root);
    match docs_rule(dir, &root, &main_root, &config) {
        Some(allowed) => {
            println!(
                "  새 문서 파일: 허용 경로 밖은 막음 ({})",
                allowed.join(", ")
            );
            println!("  규칙 파일 변경: 훅에서 한 번 알림");
        }
        None => println!("  새 문서 파일, 규칙 파일: 검사 안 함 (docs 설정 없음)"),
    }
    let (tests_in_hook, tests_source) = hook_tests(dir, &root, &main_root, &config);
    println!(
        "  훅에서 테스트: {} ({tests_source})",
        if tests_in_hook {
            "돌림"
        } else {
            "안 돌림"
        }
    );
    let plan = checks::plan(dir, None, &venvs, &checks::search_path(), Tests::Full);
    for check in &plan.checks {
        let is_test = matches!(check.name.as_str(), "pytest" | "cargo test");
        let when = if is_test && !tests_in_hook {
            "check에서만"
        } else {
            "훅과 check"
        };
        println!("  {}: {} ({when})", check.name, check.tool);
    }
    let custom = match config::custom_checks(dir) {
        Ok(list) => list,
        Err(error) => {
            println!("  직접 정한 검사: 설정 오류로 무시함 ({error})");
            Vec::new()
        }
    };
    for c in &custom {
        let when = if c.hook {
            "훅과 check"
        } else {
            "check에서만"
        };
        let exts = c.exts.as_ref().map_or(String::new(), |e| {
            format!(", {} 파일이 바뀔 때", e.join(", "))
        });
        println!("  {}: {} ({when}{exts})", c.name, c.run);
    }
    for name in &plan.missing {
        println!("  {name}: 설정은 있지만 도구를 못 찾아 건너뜀");
    }
    if plan.checks.is_empty() && plan.missing.is_empty() && custom.is_empty() {
        println!("  설정된 검사 도구가 없습니다");
    }
    ExitCode::SUCCESS
}

fn session_files(dirty: &[PathBuf], snapshot: &HashMap<PathBuf, String>) -> Vec<(PathBuf, String)> {
    dirty
        .iter()
        .map(|f| (f.clone(), state::content_hash(f)))
        .filter(|(f, digest)| snapshot.get(f) != Some(digest))
        .collect()
}

fn with_event(mut entry: Value, event: &str) -> Value {
    entry["event"] = json!(event);
    entry
}

fn hook_stop(root: &Path, session: &str, state: &State) {
    let snapshot = state.load_snapshot(root).unwrap_or_default();
    let mut changed = session_files(&git::dirty_files(root), &snapshot);
    if let Some(touched) = state.load_touched(root) {
        changed.retain(|(f, _)| touched.contains(f));
    }
    if changed.is_empty() {
        return;
    }
    let mut stamp: Vec<String> = changed
        .iter()
        .map(|(f, d)| format!("{}:{d}", f.display()))
        .collect();
    stamp.sort();
    stamp.push(git::head(root));
    let fingerprint = state::hash(&stamp.iter().map(String::as_str).collect::<Vec<_>>());
    if State::read(&state.passed) == fingerprint {
        return;
    }

    let files: Vec<PathBuf> = changed.into_iter().map(|(f, _)| f).collect();
    let baseline = Baseline::Session {
        state,
        snapshot: &snapshot,
    };
    let mut report = evaluate(root, &files, false, &baseline, true);
    report
        .failures
        .extend(rule_notice(root, state, &report.rules));

    let failed: Vec<Value> = report
        .failures
        .iter()
        .map(|f| {
            let project = f.project.strip_prefix(root).unwrap_or(&f.project);
            json!({ "project": project.display().to_string(), "check": f.check, "count": f.count })
        })
        .collect();
    let base = json!({
        "session": session,
        "root": root.display().to_string(),
        "files": files.len(),
        "failed": failed,
        "skipped": report.skipped,
    });
    let skip_note = (!report.skipped.is_empty() && !state.noticed.exists()).then(|| {
        let _ = fs::write(&state.noticed, "");
        format!(
            "실행하지 못해 건너뛴 검사: {} (이 대화에서는 다시 알리지 않음, 자세히: kiln status)",
            report.skipped.join(", ")
        )
    });

    if report.failures.is_empty() {
        let _ = fs::write(&state.passed, &fingerprint);
        let _ = fs::remove_file(&state.blocks);
        state.log(with_event(base, "pass"));
        if let Some(note) = skip_note {
            println!("{}", json!({ "systemMessage": note }));
        }
        return;
    }

    let count = State::read(&state.blocks)
        .trim()
        .parse::<u32>()
        .unwrap_or(0)
        + 1;
    if count > MAX_BLOCKS {
        let _ = fs::remove_file(&state.blocks);
        state.log(with_event(base, "giveup"));
        let message = format!("kiln 검사가 {MAX_BLOCKS}번 연속 실패한 채 답변이 끝났습니다");
        println!("{}", json!({ "systemMessage": message }));
        return;
    }
    let _ = fs::write(&state.blocks, count.to_string());
    state.log(with_event(base, "block"));
    let reports: Vec<&str> = report.failures.iter().map(|f| f.report.as_str()).collect();
    let reason = format!(
        "kiln 검사가 실패했습니다. 원인을 고친 뒤 끝내세요. 검사를 끄거나 규칙을 무시하는 표시로 넘기지 마세요. 정당한 예외라고 판단되면 넘기지 말고 사용자에게 먼저 확인하세요.\n\n{}",
        reports.join("\n\n")
    );
    let mut out = json!({ "decision": "block", "reason": reason });
    if let Some(note) = skip_note {
        out["systemMessage"] = json!(note);
    }
    println!("{out}");
}

fn track(root: &Path, cwd: &Path, payload: &Value, state: &State) {
    let id = payload["tool_use_id"].as_str().unwrap_or("unknown");
    let dirty = git::dirty_files(root);
    if payload["hook_event_name"] == "PreToolUse" {
        state.open_window(id, root, &dirty);
        return;
    }
    let mut changed = state.close_window(id, root, &dirty);
    for key in ["file_path", "notebook_path"] {
        if let Some(named) = payload["tool_input"][key].as_str() {
            let path = cwd.join(named);
            changed.push(fs::canonicalize(&path).unwrap_or(path));
        }
    }
    state.add_touched(root, &changed);
}

fn hook(mode: &str) -> ExitCode {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let cwd = payload["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_default();
    let session = payload["session_id"].as_str().unwrap_or("unknown");
    let Some(root) = git::toplevel(&cwd) else {
        return ExitCode::SUCCESS;
    };
    let state = State::new(State::base_dir(), session, &root);
    match mode {
        "start" => {
            state.clean_old();
            state.save_snapshot(&root, &git::dirty_files(&root));
        }
        "stop" => hook_stop(&root, session, &state),
        "track" => track(&root, &cwd, &payload, &state),
        _ => return usage(),
    }
    ExitCode::SUCCESS
}

fn check(args: &[String]) -> ExitCode {
    let cwd = env::current_dir().unwrap_or_default();
    let Some(root) = git::toplevel(&cwd) else {
        eprintln!("git 저장소 안에서 실행하세요");
        return ExitCode::from(2);
    };
    let base;
    let (files, baseline, whole) = match args.first().map(String::as_str) {
        Some("--staged") => (scope::staged_files(&root), Baseline::Index, false),
        Some("--base") => {
            let Some(reference) = args.get(1) else {
                return usage();
            };
            let Some(found) = scope::merge_base(&root, reference) else {
                eprintln!("{reference}와 HEAD의 공통 조상을 찾지 못했습니다");
                return ExitCode::from(2);
            };
            base = found;
            (
                scope::files_since(&root, &base),
                Baseline::Ref(base.clone()),
                false,
            )
        }
        Some("--all") => (scope::tracked_files(&root), Baseline::Everything, true),
        _ => return usage(),
    };
    let report = evaluate(&root, &files, whole, &baseline, false);
    for note in &report.skipped {
        eprintln!("kiln: 건너뜀: {note}");
    }
    for failure in &report.failures {
        eprintln!("{}\n", failure.report);
    }
    if !report.existing.is_empty() {
        eprintln!("정리할 기존 표시 (실패로 치지 않음)\n");
    }
    for item in &report.existing {
        eprintln!("{}\n", item.report);
    }
    if report.failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn commit_msg(args: &[String]) -> ExitCode {
    let Some(file) = args.first() else {
        return usage();
    };
    let message = fs::read_to_string(file).unwrap_or_default();
    let errors = commit_msg::check(&message);
    for e in &errors {
        eprintln!("커밋 메시지 오류: {e}");
    }
    if errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn marks(args: &[String]) -> ExitCode {
    for file in args {
        let lines: Vec<String> = comments::marks(Path::new(file))
            .into_iter()
            .map(|(n, _)| n.to_string())
            .collect();
        println!("{file}\t{}", lines.join(","));
    }
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("check") => check(rest),
        Some("commit-msg") => commit_msg(rest),
        Some("hook") => hook(rest.first().map_or("", String::as_str)),
        Some("stats") => {
            stats::print(&State::base_dir());
            ExitCode::SUCCESS
        }
        Some("marks") => marks(rest),
        Some("status") => status(),
        Some("--version") => {
            println!("kiln {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
