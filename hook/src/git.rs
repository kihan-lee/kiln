use std::path::{Path, PathBuf};
use std::process::Command;

pub fn git(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

pub fn toplevel(cwd: &Path) -> Option<PathBuf> {
    let top = git(cwd, &["rev-parse", "--show-toplevel"]);
    let top = top.trim();
    (!top.is_empty()).then(|| PathBuf::from(top))
}

pub fn main_worktree(root: &Path) -> PathBuf {
    let common = git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    Path::new(common.trim())
        .parent()
        .map_or_else(|| root.to_path_buf(), Path::to_path_buf)
}

pub fn head(root: &Path) -> String {
    git(root, &["rev-parse", "HEAD"]).trim().to_string()
}

pub fn dirty_files(root: &Path) -> Vec<PathBuf> {
    git(
        root,
        &[
            "status",
            "--porcelain",
            "-z",
            "--no-renames",
            "--untracked-files=all",
        ],
    )
    .split('\0')
    .filter(|e| e.len() > 3)
    .map(|e| root.join(&e[3..]))
    .collect()
}

pub fn is_tracked(root: &Path, file: &Path) -> bool {
    let name = file.to_string_lossy();
    !git(root, &["ls-files", "--", &name]).trim().is_empty()
}

pub fn added_lines_from_diff(diff: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    for header in diff.lines().filter(|l| l.starts_with("@@")) {
        let Some(plus) = header.split_whitespace().find(|p| p.starts_with('+')) else {
            continue;
        };
        let mut parts = plus[1..].split(',');
        let start: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let count: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
        lines.extend(start..start + count);
    }
    lines
}

pub fn added_since_head(root: &Path, file: &Path) -> Vec<usize> {
    let name = file.to_string_lossy();
    added_lines_from_diff(&git(root, &["diff", "HEAD", "-U0", "--", &name]))
}

pub fn added_since_copy(root: &Path, before: &Path, after: &Path) -> Vec<usize> {
    let diff = git(
        root,
        &[
            "diff",
            "--no-index",
            "-U0",
            "--",
            &before.to_string_lossy(),
            &after.to_string_lossy(),
        ],
    );
    added_lines_from_diff(&diff)
}

#[cfg(test)]
mod tests {
    use super::added_lines_from_diff;

    #[test]
    fn parses_hunk_headers() {
        let diff = "@@ -1,0 +2,3 @@\n+a\n@@ -9 +12 @@\n-x\n+y\n@@ -20,2 +24,0 @@\n";
        assert_eq!(added_lines_from_diff(diff), vec![2, 3, 4, 12]);
    }
}
