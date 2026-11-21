import re
import sys
from pathlib import Path

TYPES = (
    "feat",
    "fix",
    "refactor",
    "docs",
    "test",
    "chore",
    "perf",
    "build",
    "ci",
    "style",
    "revert",
)
HEADER = re.compile(rf"^({'|'.join(TYPES)})(\([^)]+\))?!?: \S")
AUTO = ("Merge ", "Revert ", "fixup! ", "squash! ", "amend! ")


def check(message: str) -> list[str]:
    lines = [ln for ln in message.splitlines() if not ln.startswith("#")]
    while lines and not lines[0].strip():
        lines.pop(0)
    if not lines:
        return ["커밋 메시지가 비어 있습니다"]
    errors = []
    header = lines[0]
    if not header.startswith(AUTO) and not HEADER.match(header):
        errors.append(
            f"첫 줄은 '타입: 요약' 형식이어야 합니다 (타입: {', '.join(TYPES)}). 현재: {header!r}"
        )
    if any(re.match(r"^\s*co-authored-by:", ln, re.IGNORECASE) for ln in lines):
        errors.append("공동 작성자(Co-Authored-By) 줄은 넣지 않습니다")
    return errors


def main() -> int:
    errors = check(Path(sys.argv[1]).read_text(encoding="utf-8"))
    for e in errors:
        print(f"커밋 메시지 오류: {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
