use crate::scope::Baseline;
use std::path::{Path, PathBuf};

const RULE_FILES: [&str; 3] = ["CLAUDE.md", "CLAUDE.local.md", "AGENTS.md"];

fn is_markdown(file: &Path) -> bool {
    file.extension().is_some_and(|x| x == "md")
}

pub fn is_rule_file(rel: &Path) -> bool {
    let named = rel
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| RULE_FILES.contains(&n));
    named || (is_markdown(rel) && rel.components().any(|c| c.as_os_str() == ".claude"))
}

pub fn rule_files(dir: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    files
        .iter()
        .filter(|f| is_rule_file(f.strip_prefix(dir).unwrap_or(f)))
        .cloned()
        .collect()
}

pub fn stray(
    root: &Path,
    dir: &Path,
    files: &[PathBuf],
    baseline: &Baseline,
    allowed: &[String],
) -> Vec<String> {
    files
        .iter()
        .filter(|f| is_markdown(f) && f.is_file())
        .filter_map(|f| f.strip_prefix(dir).ok().map(|rel| (f, rel)))
        .filter(|(_, rel)| !is_rule_file(rel))
        .filter(|(_, rel)| !allowed.iter().any(|a| rel.starts_with(a)))
        .filter(|(f, _)| baseline.is_new(root, f))
        .map(|(_, rel)| rel.display().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::is_rule_file;
    use std::path::Path;

    #[test]
    fn recognizes_rule_files() {
        assert!(is_rule_file(Path::new("CLAUDE.md")));
        assert!(is_rule_file(Path::new("pkg/AGENTS.md")));
        assert!(is_rule_file(Path::new(".claude/skills/x/SKILL.md")));
        assert!(!is_rule_file(Path::new(".claude/settings.json")));
        assert!(!is_rule_file(Path::new("docs/claude.md")));
    }
}
