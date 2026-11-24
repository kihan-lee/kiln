use crate::git;
use crate::state::State;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub enum Baseline<'a> {
    Session {
        state: &'a State,
        snapshot: &'a HashMap<PathBuf, String>,
    },
    Index,
    Ref(String),
    Everything,
}

impl Baseline<'_> {
    pub fn added(&self, root: &Path, file: &Path) -> Option<Vec<usize>> {
        let name = file.to_string_lossy();
        match self {
            Baseline::Session { state, snapshot } => {
                if let Some(copy) = snapshot.get(file).and_then(|d| state.snapshot_copy(d)) {
                    return Some(git::added_since_copy(root, &copy, file));
                }
                git::is_tracked(root, file).then(|| git::added_since_head(root, file))
            }
            Baseline::Index => Some(git::added_lines_from_diff(&git::git(
                root,
                &["diff", "--cached", "-U0", "--", &name],
            ))),
            Baseline::Ref(base) => git::is_tracked(root, file).then(|| {
                git::added_lines_from_diff(&git::git(root, &["diff", base, "-U0", "--", &name]))
            }),
            Baseline::Everything => None,
        }
    }
}

pub fn staged_files(root: &Path) -> Vec<PathBuf> {
    split_names(
        root,
        &git::git(
            root,
            &[
                "diff",
                "--cached",
                "--name-only",
                "-z",
                "--diff-filter=ACMR",
            ],
        ),
    )
}

pub fn files_since(root: &Path, base: &str) -> Vec<PathBuf> {
    split_names(
        root,
        &git::git(
            root,
            &["diff", "--name-only", "-z", "--diff-filter=ACMR", base],
        ),
    )
}

pub fn tracked_files(root: &Path) -> Vec<PathBuf> {
    split_names(root, &git::git(root, &["ls-files", "-z"]))
}

pub fn merge_base(root: &Path, reference: &str) -> Option<String> {
    let base = git::git(root, &["merge-base", reference, "HEAD"]);
    let base = base.trim();
    (!base.is_empty()).then(|| base.to_string())
}

fn split_names(root: &Path, output: &str) -> Vec<PathBuf> {
    output
        .split('\0')
        .filter(|n| !n.is_empty())
        .map(|n| root.join(n))
        .collect()
}
