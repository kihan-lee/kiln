import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

BATCH = 500
EXTS = {".cu", ".cuh", ".c", ".cc", ".cpp", ".cxx", ".h", ".hpp"}
LOC = re.compile(r"Loc=<[^\n>]*:(\d+):\d+>\n?")
UNCLEAN = "[UnClean='"


def is_doc(text: str) -> bool:
    if text.startswith(("///", "//!", "/*!")):
        return True
    return text.startswith("/**") and not text.startswith("/**/")


def reference(path: Path) -> list[int] | None:
    out = subprocess.run(
        ["clang", "-x", "c++", "-std=c++17", "-fsyntax-only", "-Xclang", "-dump-raw-tokens", str(path)],
        capture_output=True,
        text=True,
        errors="replace",
    ).stderr
    lines = set()
    parts = LOC.split(out)
    for record, start in zip(parts[0::2], parts[1::2]):
        record = record.lstrip("\n")
        if not record.startswith("comment '"):
            continue
        if UNCLEAN in record:
            text = record[record.index(UNCLEAN) + len(UNCLEAN) : record.rindex("']")]
        else:
            text = record[len("comment '") : record.index("'\t")]
        if is_doc(text):
            continue
        start = int(start)
        if text.startswith("//"):
            lines.add(start)
            continue
        lines.update(start + k for k, part in enumerate(text.split("\n")) if part.strip())
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


def pragma_lines(path: Path) -> set[int]:
    text = path.read_text(encoding="utf-8", errors="replace")
    return {n for n, line in enumerate(text.splitlines(), 1) if line.lstrip().startswith("#")}


def main() -> int:
    kiln, *roots = sys.argv[1:]
    files = sorted(
        f for root in roots for f in Path(root).rglob("*") if f.suffix in EXTS and f.is_file()
    )
    got = kiln_marks(kiln, files)
    with ThreadPoolExecutor() as pool:
        expected_all = list(pool.map(reference, files))
    mismatched = []
    for f, expected in zip(files, expected_all):
        actual = set(got.get(str(f), [])) - (pragma_lines(f) - set(expected))
        if sorted(actual) != expected:
            mismatched.append((f, sorted(actual - set(expected)), sorted(set(expected) - actual)))
    print(f"비교 {len(files)}개, 불일치 {len(mismatched)}개")
    for f, extra, missing in mismatched[:30]:
        print(f"{f}\n  kiln에만: {extra[:10]}\n  clang에만: {missing[:10]}")
    return 1 if mismatched else 0


if __name__ == "__main__":
    sys.exit(main())
