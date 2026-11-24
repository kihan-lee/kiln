use crate::scope::Baseline;
use std::fs;
use std::path::{Path, PathBuf};

const DIRECTIVES: [&str; 6] = ["pragma", "fmt:", "type:", "ruff:", "coding", "clang-format"];
const SUPPRESSIONS: [&str; 8] = [
    "nolint",
    "noqa",
    "type: ignore",
    "ty: ignore",
    "pyright: ignore",
    "mypy: ignore",
    "ruff: noqa",
    "pylint: disable",
];
const RUST_SUPPRESSIONS: [&str; 4] = ["#[allow(", "#![allow(", "#[expect(", "#![expect("];
const C_SUPPRESSIONS: [&str; 5] = [
    "#pragma nv_diag_suppress",
    "#pragma diag_suppress",
    "#pragma GCC diagnostic ignored",
    "#pragma clang diagnostic ignored",
    "#pragma warning(disable",
];
pub const C_EXTS: [&str; 8] = ["cu", "cuh", "c", "cc", "cpp", "cxx", "h", "hpp"];

#[derive(Debug, PartialEq)]
pub enum Kind {
    Comment,
    Suppression,
}

pub struct Finding {
    pub kind: Kind,
    pub text: String,
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

fn word_before(chars: &[char], i: usize) -> String {
    let start = chars[..i]
        .iter()
        .rposition(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map_or(0, |p| p + 1);
    chars[start..i].iter().collect()
}

fn line_end(chars: &[char], from: usize) -> usize {
    let mut j = from;
    while j < chars.len() && chars[j] != '\n' {
        if chars[j] == '\\' && chars.get(j + 1) == Some(&'\n') {
            j += 2;
        } else {
            j += 1;
        }
    }
    j
}

pub fn c_marks(src: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = src.chars().collect();
    let mut found = Vec::new();
    let (mut i, mut line, mut line_start) = (0, 1, true);
    let newlines = |a: usize, b: usize| chars[a..b].iter().filter(|&&c| c == '\n').count();
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\n' {
            line += 1;
            line_start = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let first_on_line = std::mem::replace(&mut line_start, false);
        if c == '#' && first_on_line {
            let text: String = chars[i..line_end(&chars, i)].iter().collect();
            let words: Vec<&str> = text[1..].split_whitespace().collect();
            let normal = format!("#{}", words.join(" "));
            if C_SUPPRESSIONS.iter().any(|p| normal.starts_with(p)) {
                found.push((line, normal));
            }
            i += 1;
        } else if c == '/' && next == Some('/') {
            let end = line_end(&chars, i);
            let text: String = chars[i..end].iter().collect();
            let text = text.lines().next().unwrap_or("").trim().to_string();
            if !(text.starts_with("///") || text.starts_with("//!")) {
                found.push((line, text));
            }
            line += newlines(i, end);
            i = end;
        } else if c == '/' && next == Some('*') {
            let third = chars.get(i + 2).copied();
            let doc = third == Some('!') || (third == Some('*') && chars.get(i + 3) != Some(&'/'));
            let end = (i + 2..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == '*' && chars[j + 1] == '/')
                .map_or(chars.len(), |j| j + 2);
            if !doc {
                let body: String = chars[i..end].iter().collect();
                for (k, part) in body.split('\n').enumerate() {
                    if !part.trim().is_empty() {
                        found.push((line + k, part.trim().to_string()));
                    }
                }
            }
            line += newlines(i, end);
            i = end;
        } else if c == '"'
            && matches!(
                word_before(&chars, i).as_str(),
                "R" | "LR" | "uR" | "UR" | "u8R"
            )
        {
            let open = (i + 1..chars.len()).find(|&j| chars[j] == '(' || chars[j] == '\n');
            let Some(open) = open.filter(|&j| chars[j] == '(') else {
                i += 1;
                continue;
            };
            let close: Vec<char> = std::iter::once(')')
                .chain(chars[i + 1..open].iter().copied())
                .chain(std::iter::once('"'))
                .collect();
            let end = (open + 1..chars.len())
                .find(|&j| chars[j..].starts_with(&close))
                .map_or(chars.len(), |j| j + close.len());
            line += newlines(i, end);
            i = end;
        } else if c == '"' || c == '\'' {
            let before = word_before(&chars, i);
            let separator = c == '\''
                && !before.is_empty()
                && !matches!(before.as_str(), "L" | "u" | "U" | "u8");
            if separator {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c && chars[j] != '\n' {
                if chars[j] == '\\' {
                    line += usize::from(chars.get(j + 1) == Some(&'\n'));
                    j += 2;
                } else {
                    j += 1;
                }
            }
            let closed = chars.get(j) == Some(&c);
            i = if closed { j + 1 } else { j.min(chars.len()) };
        } else {
            i += 1;
        }
    }
    found
}

pub fn classify(text: &str, line: usize) -> Option<Kind> {
    let pragma = C_SUPPRESSIONS.iter().any(|p| text.starts_with(p));
    if pragma || text.starts_with("#[") || text.starts_with("#![") {
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

pub fn marks(file: &Path) -> Vec<(usize, String)> {
    let src = fs::read_to_string(file).unwrap_or_default();
    match file.extension().and_then(|x| x.to_str()) {
        Some("py") => python_marks(&src),
        Some("rs") => rust_marks(&src),
        Some(x) if C_EXTS.contains(&x) => c_marks(&src),
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
    use super::{c_marks, classify, python_marks, rust_marks, Kind};

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
    fn c_marks_find_comments_outside_strings_and_skip_doc_comments() {
        let src = concat!(
            "// plain\n",
            "/// doc\n",
            "int x = 1; // trailing\n",
            "/* block\n",
            "   more */\n",
            "/** doxygen */\n",
            "const char* s = \"// not /* not */\";\n",
            "auto r = R\"x(/* no \" // no)x\";\n",
            "int n = 1'000'000; // sep\n",
            "char c = u'/'; // after char\n",
            "#pragma unroll\n",
            "#  pragma nv_diag_suppress 177\n",
            "// clang-format off\n",
            "int y; // NOLINT(bugprone)\n",
            "// cont \\\n",
            "still comment\n",
            "int z = '\\''; /**/\n",
        );
        let found = c_marks(src);
        let lines: Vec<usize> = found.iter().map(|(n, _)| *n).collect();
        assert_eq!(lines, vec![1, 3, 4, 5, 9, 10, 12, 13, 14, 15, 17]);
        let kinds: Vec<Option<Kind>> = found.iter().map(|(n, t)| classify(t, *n)).collect();
        assert_eq!(kinds[6], Some(Kind::Suppression));
        assert_eq!(kinds[7], None);
        assert_eq!(kinds[8], Some(Kind::Suppression));
        assert_eq!(kinds[0], Some(Kind::Comment));
    }

    #[test]
    fn rust_marks_skip_doc_comments() {
        let src = "/// doc\n//! inner\n// plain\nfn f() {}\n    #[allow(unused)]\n";
        let found: Vec<usize> = rust_marks(src).into_iter().map(|(n, _)| n).collect();
        assert_eq!(found, vec![3, 5]);
    }
}
