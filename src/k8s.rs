use crate::checks::{check, find_on_path, Check};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const YAMLLINT: [&str; 3] = [".yamllint", ".yamllint.yaml", ".yamllint.yml"];
const KUBE_LINTER: [&str; 2] = [".kube-linter.yaml", ".kube-linter.yml"];
const LINT_DEFAULT: &str = "--exclude no-read-only-root-fs,run-as-non-root";
const KUSTOMIZATION: [&str; 3] = ["kustomization.yaml", "kustomization.yml", "Kustomization"];
const YAML: [&str; 2] = ["yaml", "yml"];
const SHELL: [&str; 2] = ["sh", "bash"];

const RENDER: &str = r#"tool=$1
cache=$2
kustomize=$3
lint=$4
shift 4
mkdir -p "$cache"
err=$(mktemp)
trap 'rm -f "$err"' EXIT
fail=0
while [ $# -gt 0 ]; do
  kind=$1
  target=$2
  shift 2
  case $kind in
    k) out=$($kustomize "$target" 2>"$err") ;;
    c) out=$(helm template kiln "$target" 2>"$err") ;;
    f) out=$(cat "$target" 2>"$err") ;;
  esac || { echo "$target: 그리기 실패"; cat "$err"; fail=1; continue; }
  case $tool in
    kubeconform) res=$(printf '%s\n' "$out" | kubeconform -strict -ignore-missing-schemas -cache "$cache" - 2>&1) ;;
    kube-linter) res=$(printf '%s\n' "$out" | kube-linter lint $lint - 2>&1) ;;
  esac || { echo "$target:"; echo "$res"; fail=1; }
done
exit $fail
"#;

#[derive(PartialEq)]
enum Unit {
    Kustomize(String),
    Chart(String),
    Manifest(String),
}

fn has(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|n| dir.join(n).is_file())
}

fn ext_in(f: &Path, exts: &[&str]) -> bool {
    f.extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| exts.contains(&x))
}

fn rel(dir: &Path, p: &Path) -> String {
    let r = p
        .strip_prefix(dir)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned();
    if r.is_empty() {
        ".".into()
    } else {
        r
    }
}

fn is_manifest(f: &Path) -> bool {
    let text = fs::read_to_string(f).unwrap_or_default();
    let starts = |key: &str| text.lines().any(|l| l.starts_with(key));
    starts("kind:") && starts("apiVersion:")
}

fn includes(kustomize_dir: &Path, file: &Path) -> bool {
    let Some(first) = file
        .strip_prefix(kustomize_dir)
        .ok()
        .and_then(|r| r.components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
    else {
        return false;
    };
    if KUSTOMIZATION.contains(&first.as_str()) {
        return true;
    }
    KUSTOMIZATION.iter().any(|n| {
        fs::read_to_string(kustomize_dir.join(n))
            .unwrap_or_default()
            .lines()
            .map(|l| l.split('#').next().unwrap_or_default())
            .any(|l| l.contains(first.as_str()))
    })
}

fn unit(dir: &Path, file: &Path) -> Option<Unit> {
    for d in file.ancestors().skip(1) {
        if !d.starts_with(dir) {
            break;
        }
        if d.join("Chart.yaml").is_file() {
            return Some(Unit::Chart(rel(dir, d)));
        }
        if has(d, &KUSTOMIZATION) && includes(d, file) {
            return Some(Unit::Kustomize(rel(dir, d)));
        }
        if d == dir {
            break;
        }
    }
    (ext_in(file, &YAML) && file.is_file() && is_manifest(file))
        .then(|| Unit::Manifest(rel(dir, file)))
}

fn cache_dir() -> String {
    let base = env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env::var("HOME").unwrap_or_default()).join(".cache"));
    base.join("kiln")
        .join("kubeconform")
        .to_string_lossy()
        .into_owned()
}

pub fn plan(
    dir: &Path,
    files: Option<&[PathBuf]>,
    path: &str,
    checks: &mut Vec<Check>,
    missing: &mut Vec<String>,
) {
    let whole = files.is_none();
    let tracked: Vec<PathBuf>;
    let files = match files {
        Some(files) => files,
        None => {
            tracked = crate::git::git(dir, &["ls-files"])
                .lines()
                .map(|l| dir.join(l))
                .collect();
            &tracked
        }
    };
    let existing = |exts: &[&str]| -> Vec<String> {
        files
            .iter()
            .filter(|f| ext_in(f, exts) && f.is_file())
            .map(|f| rel(dir, f))
            .collect()
    };
    let found = |name: &str| find_on_path(name, path).map(|p| p.to_string_lossy().into_owned());

    if has(dir, &YAMLLINT) {
        let yaml = existing(&YAML);
        if !yaml.is_empty() {
            match found("yamllint") {
                Some(bin) => {
                    let yaml = if whole { vec![".".into()] } else { yaml };
                    let head = vec![bin, "--no-warnings".into(), "-f".into(), "parsable".into()];
                    checks.push(check("yamllint", [head, yaml].concat()));
                }
                None => missing.push("yamllint".into()),
            }
        }
    }

    let scripts = existing(&SHELL);
    if !scripts.is_empty() {
        match found("shellcheck") {
            Some(bin) => {
                let argv = [vec![bin, "--severity=warning".into()], scripts].concat();
                checks.push(check("shellcheck", argv));
            }
            None => missing.push("shellcheck".into()),
        }
    }

    let mut units: Vec<Unit> = Vec::new();
    for f in files {
        if let Some(u) = unit(dir, f) {
            if !units.contains(&u) {
                units.push(u);
            }
        }
    }
    if units.is_empty() {
        return;
    }
    let kustomize = if found("kustomize").is_some() {
        Some("kustomize build")
    } else if found("kubectl").is_some() {
        Some("kubectl kustomize")
    } else {
        None
    };
    if kustomize.is_none() && units.iter().any(|u| matches!(u, Unit::Kustomize(_))) {
        missing.push("kubectl".into());
    }
    let helm = found("helm").is_some();
    if !helm && units.iter().any(|u| matches!(u, Unit::Chart(_))) {
        missing.push("helm".into());
    }
    let mut targets = Vec::new();
    for u in &units {
        let (kind, target) = match u {
            Unit::Kustomize(t) if kustomize.is_some() => ("k", t),
            Unit::Chart(t) if helm => ("c", t),
            Unit::Manifest(t) => ("f", t),
            _ => continue,
        };
        targets.push(kind.to_string());
        targets.push(target.clone());
    }
    if targets.is_empty() {
        return;
    }
    let lint = KUBE_LINTER
        .iter()
        .find(|n| dir.join(n).is_file())
        .map_or(LINT_DEFAULT.to_string(), |n| format!("--config {n}"));
    for name in ["kubeconform", "kube-linter"] {
        let Some(bin) = found(name) else {
            missing.push(name.into());
            continue;
        };
        let mut argv: Vec<String> = ["sh", "-c", RENDER, "kiln", name]
            .map(String::from)
            .to_vec();
        argv.push(cache_dir());
        argv.push(kustomize.unwrap_or("kubectl kustomize").into());
        argv.push(lint.clone());
        argv.extend(targets.iter().cloned());
        let shown: Vec<&str> = targets
            .iter()
            .skip(1)
            .step_by(2)
            .map(String::as_str)
            .collect();
        let mut c = check(name, argv);
        c.tool = bin;
        c.shown = if shown.len() > 8 {
            format!("{name} < 대상 {}개", shown.len())
        } else {
            format!("{name} < {}", shown.join(" "))
        };
        checks.push(c);
    }
}
