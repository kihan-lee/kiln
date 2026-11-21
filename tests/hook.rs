use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Repo {
    root: PathBuf,
    state: PathBuf,
}

impl Repo {
    fn new(name: &str, manifest: (&str, &str)) -> Self {
        let base = std::env::temp_dir().join(format!("kiln-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("repo");
        fs::create_dir_all(&root).unwrap();
        let repo = Repo {
            root,
            state: base.join("state"),
        };
        repo.git(&["init", "-q"]);
        repo.write(manifest.0, manifest.1);
        repo.commit();
        repo
    }

    fn git(&self, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn commit(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", "c", "--allow-empty"]);
    }

    fn hook(&self, mode: &str, session: &str) -> Option<Value> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(["hook", mode])
            .env("XDG_STATE_HOME", &self.state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let payload = format!(
            r#"{{"cwd":"{}","session_id":"{session}"}}"#,
            self.root.display()
        );
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        (!text.trim().is_empty()).then(|| serde_json::from_str(text.trim()).unwrap())
    }

    fn kiln(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(args)
            .env("XDG_STATE_HOME", &self.state)
            .current_dir(&self.root)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn log_events(&self) -> Vec<String> {
        fs::read_to_string(self.state.join("kiln/log.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap()["event"].to_string())
            .collect()
    }
}

const PY: (&str, &str) = (
    "pyproject.toml",
    "[project]\nname = \"x\"\n\n[tool.kiln]\ncomments = false\n",
);

fn reason(out: &Option<Value>) -> String {
    out.as_ref()
        .map_or(String::new(), |v| v["reason"].to_string())
}

fn blocked(out: &Option<Value>) -> bool {
    out.as_ref().is_some_and(|v| v["decision"] == "block")
}

#[test]
fn ignores_work_that_was_dirty_before_the_session() {
    let repo = Repo::new("dirty", PY);
    repo.write("a.py", "x = 1\n");
    repo.commit();
    repo.write("a.py", "x = 1  # 사용자가 쓰던 주석\n");
    repo.write("draft.py", "# 사용자가 쓰던 새 파일\n");
    repo.hook("start", "s");

    repo.write("b.py", "y = 2\n");
    assert_eq!(repo.hook("stop", "s"), None);
}

#[test]
fn checks_only_lines_added_after_session_start() {
    let repo = Repo::new("added", PY);
    repo.write("a.py", "x = 1\n");
    repo.commit();
    repo.write("a.py", "x = 1  # 예전 주석\n");
    repo.hook("start", "s");

    repo.write("a.py", "x = 1  # 예전 주석\ny = 2  # 새 주석\n");
    let out = repo.hook("stop", "s");
    assert!(blocked(&out));
    let text = reason(&out);
    assert!(text.contains("a.py:2"), "{text}");
    assert!(!text.contains("a.py:1"), "{text}");
}

#[test]
fn blocks_suppressions_even_with_a_reason() {
    let repo = Repo::new("suppress", PY);
    repo.hook("start", "s");
    repo.write(
        "a.py",
        "#!/usr/bin/env python3\nimport os  # noqa: F401\nx = 1  # 이유: 외부 API 제약\n",
    );
    let out = repo.hook("stop", "s");
    let text = reason(&out);
    assert!(text.contains("검사를 끄는 표시"), "{text}");
    assert!(text.contains("a.py:2"), "{text}");
    assert!(!text.contains("a.py:1"), "{text}");
    assert!(!text.contains("a.py:3"), "{text}");
}

#[test]
fn passes_quietly_skips_repeats_and_logs() {
    let repo = Repo::new("pass", PY);
    repo.hook("start", "s");
    assert_eq!(repo.hook("stop", "s"), None);

    repo.write("a.py", "x = 1\n");
    assert_eq!(repo.hook("stop", "s"), None);
    assert_eq!(repo.hook("stop", "s"), None);
    assert_eq!(repo.log_events(), vec!["\"pass\""]);
}

#[test]
fn gives_up_after_three_blocks_and_records_recovery() {
    let repo = Repo::new("giveup", PY);
    repo.hook("start", "s");
    repo.write("a.py", "x = 1  # 설명\n");
    for _ in 0..3 {
        assert!(blocked(&repo.hook("stop", "s")));
    }
    let out = repo.hook("stop", "s").unwrap();
    assert!(out["systemMessage"].to_string().contains("연속 실패"));

    repo.write("a.py", "x = 1\n");
    assert_eq!(repo.hook("stop", "s"), None);
    assert_eq!(
        repo.log_events(),
        vec![
            "\"block\"",
            "\"block\"",
            "\"block\"",
            "\"giveup\"",
            "\"pass\""
        ]
    );
}

#[test]
fn blocks_rust_comments_and_allow_attributes_but_not_doc_comments() {
    let repo = Repo::new(
        "rust",
        (
            "Cargo.toml",
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[package.metadata.kiln]\ncomments = false\n",
        ),
    );
    repo.hook("start", "s");
    repo.write(
        "src/lib.rs",
        "/// 문서\npub fn f() {}\n// 설명\n#[allow(dead_code)]\nfn g() {}\n",
    );
    let text = reason(&repo.hook("stop", "s"));
    assert!(text.contains("lib.rs:3"), "{text}");
    assert!(text.contains("lib.rs:4"), "{text}");
    assert!(!text.contains("lib.rs:1"), "{text}");
}

#[test]
fn reports_checks_that_could_not_run() {
    let repo = Repo::new(
        "missing",
        (
            "Cargo.toml",
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        ),
    );
    repo.hook("start", "s");
    repo.write("src/lib.rs", "pub fn f() {}\n");
    let out = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(["hook", "stop"])
        .env("XDG_STATE_HOME", &repo.state)
        .env("HOME", repo.root.join("no-home"))
        .env("PATH", "/usr/bin:/bin")
        .current_dir(Path::new(&repo.root))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("건너뛴 검사"), "{text}");
    assert!(text.contains("cargo fmt"), "{text}");
}

#[test]
fn check_staged_looks_only_at_staged_additions() {
    let repo = Repo::new("staged", PY);
    repo.write("a.py", "x = 1  # 예전 주석\n");
    repo.commit();
    repo.write("a.py", "x = 1  # 예전 주석\ny = 2\n");
    repo.write("b.py", "z = 3  # 아직 스테이징 안 함\n");
    repo.git(&["add", "a.py"]);
    assert!(repo.kiln(&["check", "--staged"]).0);

    repo.write("a.py", "x = 1  # 예전 주석\ny = 2  # noqa\n");
    repo.git(&["add", "a.py"]);
    let (ok, err) = repo.kiln(&["check", "--staged"]);
    assert!(!ok);
    assert!(err.contains("a.py:2"), "{err}");
    assert!(!err.contains("b.py"), "{err}");
}

#[test]
fn check_base_compares_against_the_merge_base() {
    let repo = Repo::new("base", PY);
    repo.git(&["branch", "-M", "main"]);
    repo.write("a.py", "x = 1  # 예전 주석\n");
    repo.commit();
    repo.git(&["checkout", "-qb", "feature"]);
    repo.write("a.py", "x = 1  # 예전 주석\ny = 2  # 새 주석\n");
    repo.commit();
    let (ok, err) = repo.kiln(&["check", "--base", "main"]);
    assert!(!ok);
    assert!(err.contains("a.py:2") && !err.contains("a.py:1"), "{err}");
    assert!(repo.kiln(&["check", "--all"]).0);
}

#[test]
fn commit_msg_rejects_bad_messages() {
    let repo = Repo::new("msg", PY);
    repo.write("good", "feat: 추가\n");
    repo.write("bad", "추가함\n");
    assert!(repo.kiln(&["commit-msg", "good"]).0);
    let (ok, err) = repo.kiln(&["commit-msg", "bad"]);
    assert!(!ok && err.contains("타입: 요약"), "{err}");
}
