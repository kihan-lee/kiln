use crate::scope::Baseline;
use std::fs;
use std::path::{Path, PathBuf};

const DIRECTIVES: [&str; 5] = ["pragma", "fmt:", "type:", "ruff:", "coding"];
const SUPPRESSIONS: [&str; 7] = [
    "noqa",
    "type: ignore",
    "ty: ignore",
    "pyright: ignore",
    "mypy: ignore",
    "ruff: noqa",
    "pylint: disable",
];
const RUST_SUPPRESSIONS: [&str; 4] = ["#[allow(", "#![allow(", "#[expect(", "#![expect("];

#[derive(Debug, PartialEq)]
pub enum Kind {
    Comment,
    Suppression,
}

pub struct Finding {
    pub kind: Kind,
    pub text: String,
}

pub fn forbids_comments(dir: &Path) -> bool {
    ["pyproject.toml", "Cargo.toml"].iter().any(|name| {
        fs::read_to_string(dir.join(name))
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .skip_while(|l| !(l.starts_with('[') && l.ends_with("kiln]")))
            .skip(1)
            .take_while(|l| !l.starts_with('['))
            .filter_map(|l| l.split_once('='))
            .any(|(key, value)| key.trim() == "comments" && value.trim() == "false")
    })
}

pub fn python_marks(src: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = src.chars().collect();
    let mut found = Vec::new();
    let (mut i, mut line, mut seen_code) = (0, 1, false);
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c == '#' {
            let end = chars[i..]
                .iter()
                .position(|&x| x == '\n')
                .map_or(chars.len(), |p| i + p);
            found.push((line, chars[i..end].iter().collect()));
            i = end;
        } else if c == '"' || c == '\'' {
            let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
            let width = if triple { 3 } else { 1 };
            let (start_line, body_start) = (line, i + width);
            let mut j = body_start;
            let closed = loop {
                match chars.get(j) {
                    None => break false,
                    Some('\\') => {
                        if chars.get(j + 1) == Some(&'\n') {
                            line += 1;
                        }
                        j += 2;
                    }
                    Some('\n') if !triple => break false,
                    Some('\n') => {
                        line += 1;
                        j += 1;
                    }
                    Some(&x) if x == c => {
                        if !triple || (chars.get(j + 1) == Some(&c) && chars.get(j + 2) == Some(&c))
                        {
                            break true;
                        }
                        j += 1;
                    }
                    Some(_) => j += 1,
                }
            };
            let body_end = j.min(chars.len());
            let after = if closed { j + width } else { body_end };
            if !seen_code {
                let rest: String = chars[after..].iter().take_while(|&&x| x != '\n').collect();
                if rest.trim().is_empty() || rest.trim().starts_with('#') {
                    let body: String = chars[body_start..body_end].iter().collect();
                    let first: String = body
                        .trim()
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(60)
                        .collect();
                    found.push((start_line, format!("파일 맨 위 설명문: {first}")));
                }
            }
            seen_code = true;
            i = after;
        } else if c.is_whitespace() {
            i += 1;
        } else {
            let prefix = chars[i..]
                .iter()
                .take_while(|x| "rRuUbBfF".contains(**x))
                .count();
            if prefix > 0 && matches!(chars.get(i + prefix), Some('"') | Some('\'')) {
                i += prefix;
            } else {
                seen_code = true;
                i += 1;
            }
        }
    }
    found
}

fn rust_marks(src: &str) -> Vec<(usize, String)> {
    src.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .filter(|(_, l)| {
            let comment = l.starts_with("//") && !l.starts_with("///") && !l.starts_with("//!");
            comment || RUST_SUPPRESSIONS.iter().any(|s| l.starts_with(s))
        })
        .collect()
}

pub fn classify(text: &str, line: usize) -> Option<Kind> {
    if text.starts_with("#[") || text.starts_with("#![") {
        return Some(Kind::Suppression);
    }
    let segments: Vec<String> = text
        .split(['#', '/'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if segments
        .iter()
        .any(|s| SUPPRESSIONS.iter().any(|p| s.starts_with(p)))
    {
        return Some(Kind::Suppression);
    }
    let body = segments.first().map_or("", String::as_str);
    let allowed =
        (line == 1 && text.starts_with("#!")) || DIRECTIVES.iter().any(|d| body.starts_with(d));
    (!allowed).then_some(Kind::Comment)
}

fn marks(file: &Path) -> Vec<(usize, String)> {
    let src = fs::read_to_string(file).unwrap_or_default();
    match file.extension().and_then(|x| x.to_str()) {
        Some("py") => python_marks(&src),
        Some("rs") => rust_marks(&src),
        _ => Vec::new(),
    }
}

pub fn new_findings(
    root: &Path,
    dir: &Path,
    files: &[PathBuf],
    baseline: &Baseline,
) -> Vec<Finding> {
    let mut found = Vec::new();
    for f in files.iter().filter(|f| f.is_file()) {
        let all = marks(f);
        if all.is_empty() {
            continue;
        }
        let added = baseline.added(root, f);
        for (n, text) in all {
            if !added.as_ref().is_none_or(|a| a.contains(&n)) {
                continue;
            }
            if let Some(kind) = classify(&text, n) {
                let shown = f.strip_prefix(dir).unwrap_or(f).display();
                found.push(Finding {
                    kind,
                    text: format!("{shown}:{n}: {text}"),
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{classify, python_marks, rust_marks, Kind};

    fn lines(src: &str) -> Vec<usize> {
        python_marks(src).into_iter().map(|(n, _)| n).collect()
    }

    #[test]
    fn finds_comments_and_module_docstring_only() {
        assert_eq!(lines("\"\"\"top\"\"\"\nx = 1  # c\n"), vec![1, 2]);
        assert_eq!(lines("# c\n'''top\nmore'''\n"), vec![1, 2]);
        assert_eq!(
            lines("x = \"#no\"\ny = '''a\n# no\n'''\n"),
            Vec::<usize>::new()
        );
        assert_eq!(
            lines("def f():\n    \"\"\"doc\"\"\"\n    return 1\n"),
            Vec::<usize>::new()
        );
        assert_eq!(
            lines("import os\n\"\"\"not top\"\"\"\n"),
            Vec::<usize>::new()
        );
        assert_eq!(lines("s = 'it\\'s # no'  # yes\n"), vec![1]);
        assert_eq!(lines("x = r\"\\\\\"  # yes\n"), vec![1]);
    }

    #[test]
    fn classifies_suppressions_reasons_and_directives() {
        assert_eq!(classify("# noqa", 3), Some(Kind::Suppression));
        assert_eq!(classify("# noqa: TID251", 3), Some(Kind::Suppression));
        assert_eq!(classify("# type: ignore[attr]", 3), Some(Kind::Suppression));
        assert_eq!(classify("# 설명  # noqa: E501", 3), Some(Kind::Suppression));
        assert_eq!(classify("# ty: ignore", 3), Some(Kind::Suppression));
        assert_eq!(classify("# 설명", 3), Some(Kind::Comment));
        assert_eq!(classify("// 설명", 3), Some(Kind::Comment));
        assert_eq!(classify("#!/usr/bin/env python3", 1), None);
        assert_eq!(classify("#!/usr/bin/env python3", 2), Some(Kind::Comment));
        assert_eq!(classify("# fmt: off", 3), None);
        assert_eq!(classify("# pragma: no cover", 3), None);
        assert_eq!(classify("#[allow(dead_code)]", 3), Some(Kind::Suppression));
    }

    #[test]
    fn rust_marks_skip_doc_comments() {
        let src = "/// doc\n//! inner\n// plain\nfn f() {}\n    #[allow(unused)]\n";
        let found: Vec<usize> = rust_marks(src).into_iter().map(|(n, _)| n).collect();
        assert_eq!(found, vec![3, 5]);
    }
}
