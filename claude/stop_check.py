#!/usr/bin/env python3
"""Claude Code 훅: 세션이 끝나기 전에 프로젝트의 표준 검사를 돌린다.

사용:
    stop_check.py start   SessionStart 훅. 세션 시작 시점의 작업 폴더 상태를 기록한다.
    stop_check.py stop    Stop 훅. 세션이 파일을 바꿨으면 scripts/check.sh --quick 을 돌린다.

동작:
- 바뀐 파일에서 위로 올라가며 가장 가까운 scripts/check.sh 를 찾아 실행한다.
  check.sh 가 없는 저장소에서는 아무 일도 하지 않는다.
- 세션이 시작된 뒤 작업 폴더가 그대로면 건너뛴다 (질문만 한 세션을 막지 않는다).
- 마지막으로 통과한 상태와 같으면 건너뛴다 (같은 검사를 반복하지 않는다).
- 검사가 통과하면 아무것도 출력하지 않는다 (모델 문맥에 들어가는 것이 없다).
- 검사가 실패하면 종료를 막고 출력 끝부분을 모델에 돌려준다.
- 한 세션에서 연속으로 MAX_BLOCKS 번 막았으면 더 막지 않고 사용자에게 알린다.
- DEV_STANDARD_SKIP=1 이면 끈다.
"""

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

MAX_BLOCKS = 3
TAIL_LINES = 40
CHECK_TIMEOUT = 540
STATE = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state")) / "dev-standard"


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=False
    ).stdout


def changed_files(root: Path) -> list[Path]:
    out = git(root, "status", "--porcelain", "-z", "--untracked-files=all")
    files = []
    entries = out.split("\0")
    i = 0
    while i < len(entries):
        entry = entries[i]
        i += 1
        if len(entry) < 4:
            continue
        status, path = entry[:2], entry[3:]
        if status[0] in "RC":
            i += 1  # 이름을 바꾼 항목은 원래 경로가 다음 칸에 온다
        files.append(root / path)
    return files


def fingerprint(root: Path, files: list[Path]) -> str:
    h = hashlib.sha256(git(root, "rev-parse", "HEAD").encode())
    for f in sorted(files):
        try:
            st = f.stat()
            h.update(f"{f}:{st.st_mtime_ns}:{st.st_size}\n".encode())
        except OSError:
            h.update(f"{f}:gone\n".encode())
    return h.hexdigest()


def check_scripts(root: Path, files: list[Path]) -> list[Path]:
    found: list[Path] = []
    for f in files:
        d = f if f.is_dir() else f.parent
        while True:
            script = d / "scripts" / "check.sh"
            if script.is_file():
                if script not in found:
                    found.append(script)
                break
            if d == root or root not in d.parents:
                break
            d = d.parent
    return found


def state_file(kind: str, key: str) -> Path:
    STATE.mkdir(parents=True, exist_ok=True)
    return STATE / f"{kind}-{hashlib.sha256(key.encode()).hexdigest()[:24]}"


def read(path: Path) -> str:
    try:
        return path.read_text()
    except OSError:
        return ""


def main() -> int:
    mode = sys.argv[1] if len(sys.argv) > 1 else "stop"
    if os.environ.get("DEV_STANDARD_SKIP") == "1":
        return 0
    try:
        payload = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError:
        payload = {}
    cwd = Path(payload.get("cwd") or os.getcwd())
    session = str(payload.get("session_id") or "unknown")

    top = git(cwd, "rev-parse", "--show-toplevel").strip()
    if not top:
        return 0
    root = Path(top)
    files = changed_files(root)
    now = fingerprint(root, files)
    baseline = state_file("baseline", f"{session}:{root}")

    if mode == "start":
        baseline.write_text(now)
        return 0

    if read(baseline) == now:
        return 0  # 이 세션은 작업 폴더를 바꾸지 않았다
    passed = state_file("passed", str(root))
    if read(passed) == now:
        return 0  # 이 상태는 이미 통과했다
    scripts = check_scripts(root, files)
    if not scripts:
        return 0

    failures = []
    for script in scripts:
        try:
            r = subprocess.run(
                ["bash", str(script), "--quick"],
                cwd=script.parent.parent,
                capture_output=True,
                text=True,
                timeout=CHECK_TIMEOUT,
                check=False,
            )
            code, out = r.returncode, (r.stdout + r.stderr)
        except subprocess.TimeoutExpired:
            code, out = 124, f"{CHECK_TIMEOUT}초 안에 끝나지 않음"
        if code != 0:
            tail = "\n".join(out.strip().splitlines()[-TAIL_LINES:])
            failures.append(f"$ bash {script} --quick  (종료 코드 {code})\n{tail}")

    blocks = state_file("blocks", f"{session}:{root}")
    if not failures:
        passed.write_text(fingerprint(root, changed_files(root)))
        blocks.unlink(missing_ok=True)
        return 0

    count = int(read(blocks) or 0) + 1
    blocks.write_text(str(count))
    if count > MAX_BLOCKS:
        blocks.unlink(missing_ok=True)
        msg = f"표준 검사가 {MAX_BLOCKS}번 연속 실패한 채 세션이 끝났습니다: " + ", ".join(
            str(s) for s in scripts
        )
        print(json.dumps({"systemMessage": msg}, ensure_ascii=False))
        return 0

    reason = (
        "표준 검사가 실패했습니다. 원인을 고친 뒤 끝내세요. "
        "검사를 끄거나 규칙을 무시하는 주석으로 넘기지 마세요. "
        "이 세션이 만든 문제가 아니면 고치지 말고 그 사실을 사용자에게 알리세요.\n\n"
        + "\n\n".join(failures)
    )
    print(json.dumps({"decision": "block", "reason": reason}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
