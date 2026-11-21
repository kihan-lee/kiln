use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Check {
    pub name: String,
    pub argv: Vec<String>,
}

pub enum Outcome {
    Passed,
    Failed(String),
    Missing,
}

pub fn projects(root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
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

pub fn search_path() -> String {
    let home = env::var("HOME").unwrap_or_default();
    format!(
        "{home}/.cargo/bin:{home}/.local/bin:{}",
        env::var("PATH").unwrap_or_default()
    )
}

pub fn find_on_path(name: &str, path: &str) -> Option<PathBuf> {
    env::split_paths(path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

fn check(name: &str, argv: Vec<String>) -> Check {
    Check {
        name: name.into(),
        argv,
    }
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
    with_tests: bool,
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
    if py.contains("[tool.ruff") && !sources.is_empty() {
        match tool("ruff") {
            Some((ruff, _)) => {
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
    if py.contains("[tool.ty") {
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
    if with_tests && py.contains("[tool.pytest") {
        match tool("pytest") {
            Some((pytest, _)) => checks.push(check("pytest", vec![pytest, "-q".into()])),
            None => missing.push("pytest".into()),
        }
    }
    if dir.join("Cargo.toml").is_file() {
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
        if with_tests {
            checks.push(check("cargo test", vec![cargo, "test".into(), "-q".into()]));
        }
    }
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
