import ast
import io
import subprocess
import sys
import tokenize
import warnings
from pathlib import Path

BATCH = 500
warnings.simplefilter("ignore", SyntaxWarning)


def reference(source: str) -> list[int] | None:
    try:
        tree = ast.parse(source)
        tokens = list(tokenize.generate_tokens(io.StringIO(source).readline))
    except (SyntaxError, tokenize.TokenError, ValueError):
        return None
    lines = {t.start[0] for t in tokens if t.type == tokenize.COMMENT}
    first = tree.body[0] if tree.body else None
    if (
        isinstance(first, ast.Expr)
        and isinstance(first.value, ast.Constant)
        and isinstance(first.value.value, str)
    ):
        lines.add(first.lineno)
    return sorted(lines)


def kiln_marks(kiln: str, files: list[Path]) -> dict[str, list[int]]:
    found = {}
    for i in range(0, len(files), BATCH):
        out = subprocess.run(
            [kiln, "marks", *map(str, files[i : i + BATCH])],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        for row in out.splitlines():
            name, _, lines = row.rpartition("\t")
            found[name] = sorted({int(n) for n in lines.split(",") if n})
    return found


def main() -> int:
    kiln, *roots = sys.argv[1:]
    files = sorted(
        f for root in roots for f in Path(root).rglob("*.py") if f.is_file()
    )
    got = kiln_marks(kiln, files)
    compared, skipped, mismatched = 0, 0, []
    for f in files:
        try:
            source = f.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            skipped += 1
            continue
        expected = reference(source)
        if expected is None:
            skipped += 1
            continue
        compared += 1
        actual = got.get(str(f), [])
        if actual != expected:
            mismatched.append((f, sorted(set(actual) - set(expected)), sorted(set(expected) - set(actual))))
    print(f"비교 {compared}개, 건너뜀 {skipped}개, 불일치 {len(mismatched)}개")
    for f, extra, missing in mismatched[:30]:
        print(f"{f}\n  kiln에만: {extra[:10]}\n  tokenize에만: {missing[:10]}")
    return 1 if mismatched else 0


if __name__ == "__main__":
    sys.exit(main())
