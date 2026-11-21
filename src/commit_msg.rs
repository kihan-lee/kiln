const TYPES: [&str; 11] = [
    "feat", "fix", "refactor", "docs", "test", "chore", "perf", "build", "ci", "style", "revert",
];
const AUTO: [&str; 5] = ["Merge ", "Revert ", "fixup! ", "squash! ", "amend! "];

fn valid_header(header: &str) -> bool {
    let kind_end = header.find(['(', '!', ':']).unwrap_or(header.len());
    if !TYPES.contains(&&header[..kind_end]) {
        return false;
    }
    let mut rest = &header[kind_end..];
    if let Some(scoped) = rest.strip_prefix('(') {
        match scoped.find(')') {
            Some(close) if close > 0 => rest = &scoped[close + 1..],
            _ => return false,
        }
    }
    rest = rest.strip_prefix('!').unwrap_or(rest);
    rest.strip_prefix(": ")
        .and_then(|summary| summary.chars().next())
        .is_some_and(|c| !c.is_whitespace())
}

pub fn check(message: &str) -> Vec<String> {
    let lines: Vec<&str> = message
        .lines()
        .filter(|l| !l.starts_with('#'))
        .skip_while(|l| l.trim().is_empty())
        .collect();
    let Some(header) = lines.first() else {
        return vec!["커밋 메시지가 비어 있습니다".into()];
    };
    let mut errors = Vec::new();
    if !AUTO.iter().any(|a| header.starts_with(a)) && !valid_header(header) {
        errors.push(format!(
            "첫 줄은 '타입: 요약' 형식이어야 합니다 (타입: {}). 현재: {header:?}",
            TYPES.join(", ")
        ));
    }
    if lines
        .iter()
        .any(|l| l.trim_start().to_lowercase().starts_with("co-authored-by:"))
    {
        errors.push("공동 작성자(Co-Authored-By) 줄은 넣지 않습니다".into());
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn accepts_conventional_headers_and_git_generated_messages() {
        for msg in [
            "feat: 평가셋 추가",
            "fix(llm): 예외 처리 보강\n\n- 본문 불릿",
            "refactor!: 구조 전환",
            "docs: 문서 갱신\n# 주석 줄은 무시\n",
            "\n\nci: 앞의 빈 줄은 무시",
            "Merge branch 'x'",
            "fixup! feat: 평가셋 추가",
        ] {
            assert_eq!(check(msg), Vec::<String>::new(), "{msg}");
        }
    }

    #[test]
    fn rejects_bad_headers() {
        for msg in [
            "평가셋 추가",
            "feature: 평가셋 추가",
            "feat:평가셋 추가",
            "feat:  평가셋 추가",
            "Feat: 평가셋 추가",
            "feat(): 빈 범위",
            "feat(llm: 닫히지 않은 범위",
            "",
            "# 주석만",
        ] {
            assert!(!check(msg).is_empty(), "{msg}");
        }
    }

    #[test]
    fn rejects_co_author_lines() {
        let errors = check("feat: 추가\n\n- 내용\n\nCo-Authored-By: Someone <a@b.c>");
        assert!(errors.iter().any(|e| e.contains("공동 작성자")));
    }
}
