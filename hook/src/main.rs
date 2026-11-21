mod checks;
mod comments;
mod git;
mod state;

use checks::Outcome;
use comments::Kind;
use serde_json::{json, Value};
use state::State;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::{env, fs};

const MAX_BLOCKS: u32 = 3;
const TAIL_LINES: usize = 40;
const FINDING_LIMIT: usize = 20;

struct Failure {
    project: PathBuf,
    check: String,
    count: usize,
    report: String,
}

fn session_files(dirty: &[PathBuf], snapshot: &HashMap<PathBuf, String>) -> Vec<(PathBuf, String)> {
    dirty
        .iter()
        .map(|f| (f.clone(), state::content_hash(f)))
        .filter(|(f, digest)| snapshot.get(f) != Some(digest))
        .collect()
}

fn findings_failure(dir: &Path, found: &[comments::Finding]) -> Vec<Failure> {
    let mut failures = Vec::new();
    for (kind, check, guide) in [
        (
            Kind::Suppression,
            "suppressions",
            "검사를 끄는 표시(noqa, type: ignore, #[allow] 등)는 쓰지 않습니다. 원인을 고치세요. 정말 필요한 예외는 줄에 붙이지 말고 설정 파일(pyproject.toml의 per-file-ignores, Cargo.toml의 [lints])에 모읍니다.",
        ),
        (
            Kind::Comment,
            "comments",
            "주석 대신 이름, 타입, 테스트로 뜻을 드러내세요. 코드로 드러낼 수 없는 이유만 '이유:'로 시작하는 한 줄 주석으로 남길 수 있습니다. 파일 맨 위 설명문은 쓰지 않습니다.",
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
        let title = match kind {
            Kind::Suppression => "새로 추가된 검사 끄기 표시",
            Kind::Comment => "새로 추가된 주석과 파일 맨 위 설명문",
        };
        let shown: Vec<&str> = hits.iter().take(FINDING_LIMIT).copied().collect();
        failures.push(Failure {
            project: dir.to_path_buf(),
            check: check.into(),
            count: hits.len(),
            report: format!(
                "{}$ {title} {}건\n{}\n{guide}",
                dir.display(),
                hits.len(),
                shown.join("\n")
            ),
        });
    }
    failures
}

fn stop(root: &Path, session: &str, state: &State) {
    let snapshot = state.load_snapshot(root).unwrap_or_default();
    let changed = session_files(&git::dirty_files(root), &snapshot);
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
    let path = checks::search_path();
    let main_root = git::main_worktree(root);
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    for dir in checks::projects(root, &files) {
        let inside: Vec<PathBuf> = files
            .iter()
            .filter(|f| f.starts_with(&dir))
            .cloned()
            .collect();
        if comments::forbids_comments(&dir) {
            let found = comments::new_findings(root, &dir, &inside, state, &snapshot);
            if !found.is_empty() {
                failures.extend(findings_failure(&dir, &found));
                continue;
            }
        }
        let mut venvs = vec![dir.join(".venv")];
        if let Ok(rel) = dir.strip_prefix(root) {
            venvs.push(main_root.join(rel).join(".venv"));
        }
        for check in checks::plan(&dir, &inside, &venvs, &path) {
            match checks::run(&check, &dir, &path, TAIL_LINES) {
                Outcome::Passed => {}
                Outcome::Missing => skipped.push(format!("{} ({})", check.name, dir.display())),
                Outcome::Failed(tail) => {
                    let shown = check
                        .argv
                        .join(" ")
                        .replace(&format!("{}/", dir.display()), "");
                    failures.push(Failure {
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

    let failed: Vec<Value> = failures
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
        "skipped": skipped,
    });
    let skip_note =
        (!skipped.is_empty()).then(|| format!("실행하지 못해 건너뛴 검사: {}", skipped.join(", ")));

    if failures.is_empty() {
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
        let message = format!("표준 검사가 {MAX_BLOCKS}번 연속 실패한 채 답변이 끝났습니다");
        println!("{}", json!({ "systemMessage": message }));
        return;
    }
    let _ = fs::write(&state.blocks, count.to_string());
    state.log(with_event(base, "block"));
    let reports: Vec<&str> = failures.iter().map(|f| f.report.as_str()).collect();
    let reason = format!(
        "표준 검사가 실패했습니다. 원인을 고친 뒤 끝내세요. 검사를 끄거나 규칙을 무시하는 표시로 넘기지 마세요.\n\n{}",
        reports.join("\n\n")
    );
    let mut out = json!({ "decision": "block", "reason": reason });
    if let Some(note) = skip_note {
        out["systemMessage"] = json!(note);
    }
    println!("{out}");
}

fn with_event(mut entry: Value, event: &str) -> Value {
    entry["event"] = json!(event);
    entry
}

fn civil_date(ts: u64) -> String {
    let days = (ts / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn stats(state_dir: &Path) {
    let text = fs::read_to_string(state_dir.join(state::LOG_FILE)).unwrap_or_default();
    let entries: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if entries.is_empty() {
        println!("기록이 없습니다");
        return;
    }
    let event = |e: &Value| e["event"].as_str().unwrap_or("").to_string();
    let count = |name: &str| entries.iter().filter(|e| event(e) == name).count();
    let ts: Vec<u64> = entries.iter().filter_map(|e| e["ts"].as_u64()).collect();
    let (first, last) = (
        ts.iter().min().copied().unwrap_or(0),
        ts.iter().max().copied().unwrap_or(0),
    );

    let mut by_check: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for e in entries
        .iter()
        .filter(|e| event(e) == "block" || event(e) == "giveup")
    {
        for f in e["failed"].as_array().into_iter().flatten() {
            let slot = by_check
                .entry(f["check"].as_str().unwrap_or("?").to_string())
                .or_default();
            slot.0 += 1;
            slot.1 += f["count"].as_u64().unwrap_or(0) as usize;
        }
    }

    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for e in &entries {
        let key = (
            e["session"].as_str().unwrap_or("").to_string(),
            e["root"].as_str().unwrap_or("").to_string(),
        );
        groups.entry(key).or_default().push(event(e));
    }
    let (mut episodes, mut recovered) = (0, 0);
    for events in groups.values() {
        let mut open = false;
        for ev in events {
            match ev.as_str() {
                "block" if !open => {
                    open = true;
                    episodes += 1;
                }
                "pass" if open => {
                    open = false;
                    recovered += 1;
                }
                "giveup" => open = false,
                _ => {}
            }
        }
    }

    println!(
        "기간: {} ~ {} (검사 {}회, 대화 {}개)",
        civil_date(first),
        civil_date(last),
        entries.len(),
        groups.len()
    );
    println!(
        "통과 {}, 막음 {}, 포기 {}",
        count("pass"),
        count("block"),
        count("giveup")
    );
    println!("막은 뒤 같은 대화에서 통과까지 간 비율: {recovered}/{episodes}");
    for (check, (times, items)) in &by_check {
        println!("  {check}: {times}회 막음, 지적 {items}건");
    }
}

fn main() {
    let mode = env::args().nth(1).unwrap_or_default();
    if mode == "stats" {
        stats(&State::base_dir());
        return;
    }
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
        return;
    };
    let state = State::new(State::base_dir(), session, &root);
    match mode.as_str() {
        "start" => {
            state.clean_old();
            state.save_snapshot(&root, &git::dirty_files(&root));
        }
        "stop" => stop(&root, session, &state),
        _ => eprintln!("사용: dev-standard-hook start|stop|stats"),
    }
}

#[cfg(test)]
mod tests {
    use super::civil_date;

    #[test]
    fn converts_unix_time_to_date() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(1_795_219_200), "2026-11-21");
    }
}
