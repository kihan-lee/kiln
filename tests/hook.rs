use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Repo {
    root: PathBuf,
    state: PathBuf,
    base: PathBuf,
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
            base,
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

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_kiln"));
        cmd.env("XDG_STATE_HOME", &self.state)
            .env("XDG_CONFIG_HOME", self.base.join("config"))
            .env_remove("VIRTUAL_ENV")
            .env_remove("CONDA_PREFIX");
        cmd
    }

    fn hook(&self, mode: &str, session: &str) -> Option<Value> {
        self.hook_with(mode, session, &[])
    }

    fn hook_with(&self, mode: &str, session: &str, envs: &[(&str, &Path)]) -> Option<Value> {
        let mut cmd = self.command();
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd
            .args(["hook", mode])
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
        let out = self
            .command()
            .args(args)
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
fn blocks_suppressions() {
    let repo = Repo::new("suppress", PY);
    repo.hook("start", "s");
    repo.write("a.py", "#!/usr/bin/env python3\nimport os  # noqa: F401\n");
    let out = repo.hook("stop", "s");
    let text = reason(&out);
    assert!(text.contains("검사를 끄는 표시"), "{text}");
    assert!(text.contains("a.py:2"), "{text}");
    assert!(!text.contains("a.py:1"), "{text}");
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
    let out = repo
        .command()
        .args(["hook", "stop"])
        .env("HOME", repo.root.join("no-home"))
        .env("PATH", "/usr/bin:/bin")
        .current_dir(Path::new(&repo.root))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("건너뛴 검사"), "{text}");
    assert!(text.contains("cargo fmt"), "{text}");

    repo.write("src/lib.rs", "pub fn g() {}\n");
    let again = repo
        .command()
        .args(["hook", "stop"])
        .env("HOME", repo.root.join("no-home"))
        .env("PATH", "/usr/bin:/bin")
        .current_dir(&repo.root)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&again.stdout).trim().is_empty());
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
fn check_all_lists_existing_marks_without_failing() {
    let repo = Repo::new("all", PY);
    repo.write("a.py", "x = 1  # 예전 주석\nimport os  # noqa: F401\n");
    repo.commit();
    let (ok, err) = repo.kiln(&["check", "--all"]);
    assert!(ok, "{err}");
    assert!(err.contains("기존 주석과 파일 맨 위 설명문 1건"), "{err}");
    assert!(err.contains("기존 검사 끄기 표시 1건"), "{err}");
    assert!(err.contains("a.py:1") && err.contains("a.py:2"), "{err}");

    let plain = Repo::new("all-off", ("pyproject.toml", "[project]\nname = \"x\"\n"));
    plain.write("a.py", "x = 1  # 예전 주석\n");
    plain.commit();
    let (ok, err) = plain.kiln(&["check", "--all"]);
    assert!(ok && !err.contains("a.py"), "{err}");
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

fn fake_tool(dir: &Path, name: &str, message: &str) {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\necho '{message}'\nexit 1\n")).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(&path, perms).unwrap();
}

#[test]
fn config_outside_the_repo_turns_on_the_comment_rule() {
    let repo = Repo::new("config", ("pyproject.toml", "[project]\nname = \"x\"\n"));
    repo.hook("start", "s");
    repo.write("a.py", "x = 1  # 설명\n");
    assert_eq!(repo.hook("stop", "s"), None);

    let config = repo.base.join("config/kiln");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        format!(
            "[[project]]\npath = \"{}\"\ncomments = false\n",
            repo.root.display()
        ),
    )
    .unwrap();
    repo.write("a.py", "x = 1  # 설명 고침\n");
    let text = reason(&repo.hook("stop", "s"));
    assert!(text.contains("a.py:1"), "{text}");
}

#[test]
fn whole_project_checks_wait_for_code_changes() {
    let repo = Repo::new(
        "gate",
        ("pyproject.toml", "[project]\nname = \"x\"\n\n[tool.ty]\n"),
    );
    repo.write("b.py", "y = 1\n");
    repo.commit();
    fake_tool(&repo.root.join(".venv/bin"), "ty", "ty 실패");
    repo.hook("start", "s");
    repo.write("notes.md", "설계 메모\n");
    assert_eq!(repo.hook("stop", "s"), None);

    repo.write("a.py", "x = 1\n");
    assert!(reason(&repo.hook("stop", "s")).contains("ty 실패"));

    repo.hook("start", "t");
    fs::remove_file(repo.root.join("b.py")).unwrap();
    assert!(reason(&repo.hook("stop", "t")).contains("ty 실패"));
}

#[test]
fn hook_runs_tests_only_when_turned_on() {
    let repo = Repo::new(
        "tests",
        (
            "pyproject.toml",
            "[project]\nname = \"x\"\n\n[tool.pytest.ini_options]\n",
        ),
    );
    fake_tool(&repo.root.join(".venv/bin"), "pytest", "pytest 실패");
    repo.hook("start", "s");
    repo.write("a.py", "x = 1\n");
    assert_eq!(repo.hook("stop", "s"), None);

    repo.write(
        "pyproject.toml",
        "[project]\nname = \"x\"\n\n[tool.pytest.ini_options]\n\n[tool.kiln]\ntests = true\n",
    );
    let text = reason(&repo.hook("stop", "s"));
    assert!(text.contains("pytest 실패"), "{text}");
    assert!(text.contains("-q -x"), "{text}");
}

#[test]
fn runs_custom_checks_by_mode_and_extension() {
    let repo = Repo::new(
        "custom",
        (
            "pyproject.toml",
            "[project]\nname = \"x\"\n\n[[tool.kiln.check]]\nname = \"spec\"\nrun = \"echo spec 실패; exit 1\"\nhook = true\nexts = [\"tla\"]\n\n[[tool.kiln.check]]\nname = \"e2e\"\nrun = \"echo e2e 실패; exit 1\"\n",
        ),
    );
    repo.hook("start", "s");
    repo.write("a.py", "x = 1\n");
    assert_eq!(repo.hook("stop", "s"), None);

    repo.write("specs/Notify.tla", "---- MODULE Notify ----\n");
    let text = reason(&repo.hook("stop", "s"));
    assert!(text.contains("spec 실패"), "{text}");

    repo.git(&["add", "a.py"]);
    let (ok, err) = repo.kiln(&["check", "--staged"]);
    assert!(!ok && err.contains("e2e 실패"), "{err}");
}

const DOCS: (&str, &str) = (
    "pyproject.toml",
    "[project]\nname = \"x\"\n\n[tool.kiln]\ndocs = [\"docs\", \"README.md\"]\n",
);

#[test]
fn blocks_new_docs_outside_allowed_paths_and_notices_rule_files_once() {
    let repo = Repo::new("docs", DOCS);
    repo.hook("start", "s");
    repo.write("docs/design.md", "설계\n");
    repo.write("README.md", "소개\n");
    assert_eq!(repo.hook("stop", "s"), None);

    repo.write("NOTES.md", "작업 메모\n");
    let text = reason(&repo.hook("stop", "s"));
    assert!(
        text.contains("NOTES.md") && text.contains("docs, README.md"),
        "{text}"
    );
    fs::remove_file(repo.root.join("NOTES.md")).unwrap();
    assert_eq!(repo.hook("stop", "s"), None);

    repo.write("CLAUDE.md", "항상 X를 하지 말 것\n");
    let text = reason(&repo.hook("stop", "s"));
    assert!(
        text.contains("규칙 파일이 바뀌었습니다") && text.contains("CLAUDE.md"),
        "{text}"
    );
    assert_eq!(repo.hook("stop", "s"), None);

    repo.write("CLAUDE.md", "항상 X를 하지 말 것\n항상 Y도\n");
    assert!(blocked(&repo.hook("stop", "s")));
}

#[test]
fn check_staged_blocks_only_new_stray_docs() {
    let repo = Repo::new("docs-staged", DOCS);
    repo.write("old.md", "예전 문서\n");
    repo.commit();
    repo.write("old.md", "예전 문서 고침\n");
    repo.write("CLAUDE.md", "규칙\n");
    repo.git(&["add", "-A"]);
    let (ok, err) = repo.kiln(&["check", "--staged"]);
    assert!(ok, "{err}");

    repo.write("SUMMARY.md", "요약\n");
    repo.git(&["add", "SUMMARY.md"]);
    let (ok, err) = repo.kiln(&["check", "--staged"]);
    assert!(
        !ok && err.contains("SUMMARY.md") && !err.contains("old.md"),
        "{err}"
    );
}

#[test]
fn docs_rule_is_off_without_the_setting() {
    let repo = Repo::new("docs-off", PY);
    repo.hook("start", "s");
    repo.write("NOTES.md", "작업 메모\n");
    repo.write("CLAUDE.md", "규칙\n");
    assert_eq!(repo.hook("stop", "s"), None);
}

#[test]
fn cuda_kernels_get_comment_checks_and_trigger_tests() {
    let repo = Repo::new(
        "cuda",
        (
            "pyproject.toml",
            "[project]\nname = \"x\"\n\n[tool.pytest.ini_options]\n\n[tool.kiln]\ncomments = false\ntests = true\n",
        ),
    );
    fake_tool(&repo.root.join(".venv/bin"), "pytest", "pytest 실패");
    repo.hook("start", "s");
    repo.write(
        "csrc/k.cu",
        "/// 문서\n__global__ void k() {} // 설명\n#pragma nv_diag_suppress 177\n",
    );
    let text = reason(&repo.hook("stop", "s"));
    assert!(text.contains("k.cu:2") && text.contains("k.cu:3"), "{text}");
    assert!(!text.contains("k.cu:1"), "{text}");

    repo.write("csrc/k.cu", "__global__ void k() {}\n");
    let text = reason(&repo.hook("stop", "s"));
    assert!(text.contains("pytest 실패"), "{text}");
}

#[test]
fn clang_format_runs_on_changed_files_when_configured() {
    let repo = Repo::new("clang", ("pyproject.toml", "[project]\nname = \"x\"\n"));
    repo.write(".clang-format", "BasedOnStyle: LLVM\n");
    repo.commit();
    let bin = repo.base.join("bin");
    fake_tool(&bin, "clang-format", "clang-format 실패");
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let envs = [("PATH", Path::new(&path))];
    repo.hook_with("start", "s", &envs);
    repo.write("a.py", "x = 1\n");
    assert_eq!(repo.hook_with("stop", "s", &envs), None);

    repo.write("k.cuh", "int f();\n");
    let text = reason(&repo.hook_with("stop", "s", &envs));
    assert!(
        text.contains("clang-format 실패") && text.contains("k.cuh"),
        "{text}"
    );
}

#[test]
fn cmake_project_root_is_the_one_that_declares_project() {
    let repo = Repo::new(
        "cmake",
        (
            "CMakeLists.txt",
            "cmake_minimum_required(VERSION 3.24)\nproject (x LANGUAGES CXX CUDA)\nadd_subdirectory(src)\n",
        ),
    );
    repo.write("src/CMakeLists.txt", "add_library(k k.cu)\n");
    repo.write("kiln.toml", "comments = false\n");
    repo.commit();
    repo.hook("start", "s");
    repo.write("src/k.cu", "__global__ void k() {} // 설명\n");
    let text = reason(&repo.hook("stop", "s"));
    assert!(text.contains("src/k.cu:1"), "{text}");
}

#[test]
fn ruff_is_found_on_path_but_ty_needs_an_environment() {
    let repo = Repo::new(
        "path",
        (
            "pyproject.toml",
            "[project]\nname = \"x\"\n\n[tool.ruff]\n\n[tool.ty]\n",
        ),
    );
    let bin = repo.base.join("bin");
    fake_tool(&bin, "ruff", "ruff 실패");
    fake_tool(&bin, "ty", "ty 실패");
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let home = repo.base.join("home");
    let envs = [("PATH", Path::new(&path)), ("HOME", home.as_path())];
    repo.hook_with("start", "s", &envs);
    repo.write("a.py", "x = 1\n");
    let out = repo.hook_with("stop", "s", &envs);
    let text = reason(&out);
    assert!(text.contains("ruff 실패"), "{text}");
    let note = out.unwrap()["systemMessage"].to_string();
    assert!(note.contains("ty"), "{note}");

    let venv = repo.base.join("active-env");
    fake_tool(&venv.join("bin"), "ty", "활성 환경의 ty 실패");
    fs::remove_file(bin.join("ruff")).unwrap();
    let envs = [
        ("PATH", Path::new(&path)),
        ("HOME", home.as_path()),
        ("VIRTUAL_ENV", venv.as_path()),
    ];
    repo.write("a.py", "x = 2\n");
    let text = reason(&repo.hook_with("stop", "s", &envs));
    assert!(text.contains("활성 환경의 ty 실패"), "{text}");
}

fn script_tool(dir: &Path, name: &str, body: &str) {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(&path, perms).unwrap();
}

#[test]
fn kubernetes_manifests_are_rendered_and_checked_by_what_changed() {
    let repo = Repo::new("k8s", (".yamllint.yaml", "extends: default\n"));
    repo.write("app/kustomization.yaml", "resources:\n  - deploy.yaml\n");
    repo.write("app/deploy.yaml", "apiVersion: v1\nkind: ConfigMap\n");
    repo.write("app/jobs/job.yaml", "apiVersion: batch/v1\nkind: Job\n");
    repo.write("chart/Chart.yaml", "name: c\n");
    repo.write("chart/templates/x.yaml", "kind: {{ .Values.k }}\n");
    repo.write("values.yaml", "replicas: 1\n");
    repo.commit();
    let bin = repo.base.join("bin");
    let log = repo.base.join("log");
    fs::create_dir_all(&log).unwrap();
    let l = log.display();
    script_tool(&bin, "yamllint", &format!("echo \"$@\" >> {l}/yamllint"));
    script_tool(&bin, "kubectl", "echo \"rendered $2\"");
    script_tool(&bin, "helm", "echo \"chart $3\"");
    script_tool(
        &bin,
        "kubeconform",
        &format!("input=$(cat)\necho \"$input\" >> {l}/kubeconform\ncase \"$input\" in *bad*) echo 'kubeconform 실패'; exit 1;; esac"),
    );
    script_tool(&bin, "kube-linter", &format!("cat >> {l}/kube-linter"));
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let home = repo.base.join("home");
    let envs = [("PATH", Path::new(&path)), ("HOME", home.as_path())];
    let seen = |name: &str| fs::read_to_string(log.join(name)).unwrap_or_default();

    repo.hook_with("start", "s", &envs);
    repo.write("values.yaml", "replicas: 2\n");
    assert_eq!(repo.hook_with("stop", "s", &envs), None);
    assert!(
        seen("yamllint").contains("values.yaml"),
        "{}",
        seen("yamllint")
    );
    assert_eq!(seen("kubeconform"), "");

    repo.write(
        "app/deploy.yaml",
        "apiVersion: v1\nkind: ConfigMap\ndata: {}\n",
    );
    repo.write("chart/templates/x.yaml", "kind: {{ .Values.kind }}\n");
    assert_eq!(repo.hook_with("stop", "s", &envs), None);
    let rendered = seen("kubeconform");
    assert!(
        rendered.contains("rendered app") && rendered.contains("chart chart"),
        "{rendered}"
    );
    assert!(!rendered.contains("Job"), "{rendered}");

    repo.write(
        "app/jobs/job.yaml",
        "apiVersion: batch/v1\nkind: Job\nmetadata:\n  name: bad\n",
    );
    let text = reason(&repo.hook_with("stop", "s", &envs));
    assert!(
        text.contains("kubeconform 실패") && text.contains("app/jobs/job.yaml"),
        "{text}"
    );
    assert_eq!(seen("kube-linter"), "");

    repo.write(".kube-linter.yaml", "checks: {}\n");
    repo.write(
        "app/jobs/job.yaml",
        "apiVersion: batch/v1\nkind: Job\nspec: {}\n",
    );
    assert_eq!(repo.hook_with("stop", "s", &envs), None);
    assert!(
        seen("kube-linter").contains("kind: Job"),
        "{}",
        seen("kube-linter")
    );
}
