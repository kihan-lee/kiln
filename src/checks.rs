use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Check {
    pub name: String,
    pub tool: String,
    pub shown: String,
    pub argv: Vec<String>,
}

pub enum Outcome {
    Passed,
    Failed(String),
    Missing,
}

fn cmake_project(dir: &Path) -> bool {
    fs::read_to_string(dir.join("CMakeLists.txt"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_ascii_lowercase())
        .any(|l| {
            l.strip_prefix("project")
                .is_some_and(|rest| rest.trim_start().starts_with('('))
        })
}

pub fn is_project(dir: &Path) -> bool {
    ["pyproject.toml", "Cargo.toml", "kiln.toml"]
        .iter()
        .chain(crate::k8s::YAMLLINT.iter())
        .any(|n| dir.join(n).is_file())
        || cmake_project(dir)
}

pub fn projects(root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for f in files {
        for dir in f.ancestors().skip(1) {
            if is_project(dir) {
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

pub fn search_path() -> String {
    let home = env::var("HOME").unwrap_or_default();
    format!(
        "{home}/.cargo/bin:{home}/.local/bin:{}",
        env::var("PATH").unwrap_or_default()
    )
}

pub fn environments(dir: &Path, root: &Path, main_root: &Path) -> Vec<PathBuf> {
    let mut found = vec![dir.join(".venv")];
    if let Ok(rel) = dir.strip_prefix(root) {
        found.push(main_root.join(rel).join(".venv"));
    }
    for var in ["VIRTUAL_ENV", "CONDA_PREFIX"] {
        if let Some(active) = env::var_os(var).filter(|v| !v.is_empty()) {
            found.push(PathBuf::from(active));
        }
    }
    found.dedup();
    found
}

pub fn find_on_path(name: &str, path: &str) -> Option<PathBuf> {
    env::split_paths(path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

pub fn check(name: &str, argv: Vec<String>) -> Check {
    Check {
        name: name.into(),
        tool: argv[0].clone(),
        shown: argv.join(" "),
        argv,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tests {
    Skip,
    Full,
    FailFast,
}

pub fn custom(files: Option<&[PathBuf]>, hook: bool, list: &[crate::config::Custom]) -> Vec<Check> {
    list.iter()
        .filter(|c| !hook || c.hook)
        .filter(|c| match (files, &c.exts) {
            (Some(files), Some(exts)) => files.iter().any(|f| {
                f.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| exts.iter().any(|e| e.trim_start_matches('.') == x))
            }),
            _ => true,
        })
        .map(|c| check(&c.name, vec!["sh".into(), "-c".into(), c.run.clone()]))
        .collect()
}

pub struct Plan {
    pub checks: Vec<Check>,
    pub missing: Vec<String>,
}

pub fn plan(
    dir: &Path,
    files: Option<&[PathBuf]>,
    venvs: &[PathBuf],
    path: &str,
    tests: Tests,
) -> Plan {
    let mut checks = Vec::new();
    let mut missing = Vec::new();
    let py = fs::read_to_string(dir.join("pyproject.toml")).unwrap_or_default();
    let text = |p: &Path| p.to_string_lossy().into_owned();
    let tool = |name: &str| {
        venvs
            .iter()
            .find(|v| v.join("bin").join(name).is_file())
            .map(|v| (text(&v.join("bin").join(name)), text(v)))
    };
    let sources: Vec<String> = match files {
        Some(files) => files
            .iter()
            .filter(|f| f.extension().is_some_and(|x| x == "py") && f.is_file())
            .filter_map(|f| f.strip_prefix(dir).ok())
            .map(text)
            .collect(),
        None => vec![".".into()],
    };
    let touches = |exts: &[&str], names: &[&str]| match files {
        None => true,
        Some(files) => files.iter().any(|f| {
            f.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| exts.contains(&x))
                || f.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| names.contains(&n))
        }),
    };
    let py_changed = touches(&["py", "pyi"], &["pyproject.toml", "uv.lock", "ty.toml"]);
    let native_changed = touches(&crate::comments::C_EXTS, &[]);
    let rs_changed = touches(
        &["rs"],
        &[
            "Cargo.toml",
            "Cargo.lock",
            "clippy.toml",
            "rustfmt.toml",
            "rust-toolchain.toml",
        ],
    );
    if py.contains("[tool.ruff") && !sources.is_empty() {
        let ruff = tool("ruff")
            .map(|(bin, _)| bin)
            .or_else(|| find_on_path("ruff", path).map(|p| text(&p)));
        match ruff {
            Some(ruff) => {
                let mut lint = vec![ruff.clone(), "check".into(), "--force-exclude".into()];
                lint.extend(sources.iter().cloned());
                checks.push(check("ruff check", lint));
                if py.contains("[tool.ruff.format]") {
                    let mut format = vec![ruff, "format".into(), "--check".into()];
                    format.push("--force-exclude".into());
                    format.extend(sources.iter().cloned());
                    checks.push(check("ruff format", format));
                }
            }
            None => missing.push("ruff".into()),
        }
    }
    if py.contains("[tool.ty") && py_changed {
        match tool("ty") {
            Some((ty, venv)) => {
                checks.push(check(
                    "ty",
                    vec![ty, "check".into(), "--python".into(), venv],
                ));
            }
            None => missing.push("ty".into()),
        }
    }
    if tests != Tests::Skip && (py_changed || native_changed) && py.contains("[tool.pytest") {
        match tool("pytest") {
            Some((pytest, _)) => {
                let mut argv = vec![pytest, "-q".into()];
                if tests == Tests::FailFast {
                    argv.push("-x".into());
                }
                checks.push(check("pytest", argv));
            }
            None => missing.push("pytest".into()),
        }
    }
    let has_style = [".clang-format", "_clang-format"]
        .iter()
        .any(|n| dir.join(n).is_file());
    if has_style {
        let native: Vec<String> = match files {
            Some(files) => files
                .iter()
                .filter(|f| {
                    f.extension()
                        .and_then(|x| x.to_str())
                        .is_some_and(|x| crate::comments::C_EXTS.contains(&x))
                        && f.is_file()
                })
                .filter_map(|f| f.strip_prefix(dir).ok())
                .map(text)
                .collect(),
            None => {
                let specs: Vec<String> = crate::comments::C_EXTS
                    .iter()
                    .map(|x| format!("*.{x}"))
                    .collect();
                let mut args = vec!["ls-files", "--"];
                args.extend(specs.iter().map(String::as_str));
                crate::git::git(dir, &args)
                    .lines()
                    .map(String::from)
                    .collect()
            }
        };
        if !native.is_empty() {
            match find_on_path("clang-format", path) {
                Some(bin) => {
                    let mut argv = vec![text(&bin), "--dry-run".into(), "--Werror".into()];
                    argv.extend(native);
                    checks.push(check("clang-format", argv));
                }
                None => missing.push("clang-format".into()),
            }
        }
    }
    if dir.join("Cargo.toml").is_file() && (rs_changed || native_changed) {
        let cargo = find_on_path("cargo", path).map_or_else(|| "cargo".into(), |p| text(&p));
        checks.push(check(
            "cargo fmt",
            vec![cargo.clone(), "fmt".into(), "--check".into()],
        ));
        let clippy = ["clippy", "--all-targets", "-q", "--", "-D", "warnings"];
        checks.push(check(
            "cargo clippy",
            std::iter::once(cargo.clone())
                .chain(clippy.map(String::from))
                .collect(),
        ));
        if tests != Tests::Skip {
            checks.push(check("cargo test", vec![cargo, "test".into(), "-q".into()]));
        }
    }
    crate::k8s::plan(dir, files, path, &mut checks, &mut missing);
    Plan { checks, missing }
}

pub fn run(check: &Check, dir: &Path, path: &str, tail_lines: usize) -> Outcome {
    let Ok(out) = Command::new(&check.argv[0])
        .args(&check.argv[1..])
        .env("PATH", path)
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .output()
    else {
        return Outcome::Missing;
    };
    if out.status.success() {
        return Outcome::Passed;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<&str> = text.trim().lines().collect();
    Outcome::Failed(lines[lines.len().saturating_sub(tail_lines)..].join("\n"))
}
