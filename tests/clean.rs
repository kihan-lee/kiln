use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("kiln-clean-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    for (rel, content) in files {
        fs::write(root.join(rel), content).unwrap();
    }
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "c"]);
    root
}

fn git(root: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn kiln(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kiln"))
        .current_dir(root)
        .env_remove("VIRTUAL_ENV")
        .env_remove("CONDA_PREFIX")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn clean_removes_comments_and_module_docstring_and_verify_accepts_it() {
    let root = repo(
        "clean",
        &[
            ("a.py", "\"\"\"모듈 설명\"\"\"\n\n# 왜\nimport os  # noqa: F401\n\n\ndef f():\n    \"\"\"함수 설명\"\"\"\n    return 1  # 하나\n"),
            ("cli.py", "\"\"\"사용법\"\"\"\nimport argparse\n\nargparse.ArgumentParser(description=__doc__)  # 설명\n"),
        ],
    );
    let out = kiln(&root, &["clean"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert_eq!(
        fs::read_to_string(root.join("a.py")).unwrap(),
        "import os\n\n\ndef f():\n    \"\"\"함수 설명\"\"\"\n    return 1\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("cli.py")).unwrap(),
        "\"\"\"사용법\"\"\"\nimport argparse\n\nargparse.ArgumentParser(description=__doc__)\n"
    );
    assert!(stdout.contains("__doc__를 써서 설명문 유지"), "{stdout}");
    assert!(kiln(&root, &["clean", "--verify"]).status.success());

    fs::write(root.join("a.py"), "import os\n\n\ndef f():\n    return 2\n").unwrap();
    let out = kiln(&root, &["clean", "--verify"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("a.py"));
}

#[test]
fn clean_refuses_files_with_uncommitted_changes() {
    let root = repo("dirty", &[("a.py", "x = 1  # c\n")]);
    fs::write(root.join("a.py"), "x = 2  # c\n").unwrap();
    let out = kiln(&root, &["clean"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        fs::read_to_string(root.join("a.py")).unwrap(),
        "x = 2  # c\n"
    );
}

#[test]
fn clean_handles_rust_and_yaml_and_lists_code_level_suppressions() {
    let root = repo(
        "langs",
        &[
            ("lib.rs", "// 왜\n/// 문서\nfn f() -> i32 {\n    1 // 하나\n}\n#[allow(dead_code)]\nfn g() {}\n"),
            ("deploy.yaml", "# 배포\nkind: Pod # 파드\nmetadata:\n  annotations:\n    ignore-check.kube-linter.io/host-ipc: \"MPS\"\n"),
        ],
    );
    let out = kiln(&root, &["clean"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert_eq!(
        fs::read_to_string(root.join("lib.rs")).unwrap(),
        "/// 문서\nfn f() -> i32 {\n    1\n}\n#[allow(dead_code)]\nfn g() {}\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("deploy.yaml")).unwrap(),
        "kind: Pod\nmetadata:\n  annotations:\n    ignore-check.kube-linter.io/host-ipc: \"MPS\"\n"
    );
    assert!(stdout.contains("lib.rs:6: #[allow(dead_code)]"), "{stdout}");
    assert!(stdout.contains("deploy.yaml:5:"), "{stdout}");
    assert!(kiln(&root, &["clean", "--verify"]).status.success());
}
