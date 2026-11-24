# kiln

에이전트가 쓴 코드를 굽는 가마. 새로 생긴 위반만 골라 막는 품질 검사 도구다. 같은 검사를 Claude Code 훅, 커밋(pre-commit), CI에서 돌린다.

원칙: 지적을 받아 고친 것은 글(규칙 문서, 메모리)로 남기기 전에 검사로 옮긴다. 순서는 린트 규칙, 타입, 테스트이고, 기계로 못 잡는 것만 글로 적는다.

## 설치

    curl -fsSL https://raw.githubusercontent.com/kihan-lee/kiln/main/scripts/install.sh | bash

`~/.local/bin/kiln`에 미리 빌드한 실행 파일을 받는다 (Linux x86_64, aarch64, macOS). 비공개 저장소일 때는 `gh`로 로그인돼 있으면 `gh`로 받는다. 버전 지정은 `KILN_VERSION=v0.1.0`.

소스에서 설치: `cargo install --path .`

## 명령

| 명령 | 쓰는 곳 | 검사 범위 |
|---|---|---|
| `kiln check --staged` | pre-commit | 커밋할 내용 |
| `kiln check --base <ref>` | CI | `<ref>`와 갈라진 뒤 바뀐 내용 |
| `kiln check --all` | 직접 실행 | 프로젝트 전체 (기존 주석과 검사 끄기 표시는 목록만 보여 주고 실패로 치지 않음) |
| `kiln commit-msg <file>` | commit-msg | 커밋 메시지 형식 |
| `kiln hook start`, `kiln hook stop` | Claude Code 훅 | 대화가 시작된 뒤 바뀐 내용 |
| `kiln status` | 직접 실행 | 지금 위치에서 어떤 검사가 어떤 도구로 도는지 |
| `kiln stats` | 직접 실행 | 훅 기록 요약 |

## 검사 내용

바뀐 파일에서 위로 올라가며 가장 가까운 `pyproject.toml`이나 `Cargo.toml`을 찾고, 그 프로젝트에 설정된 도구만 돌린다. 템플릿이 아닌 프로젝트에서도 그대로 동작한다. 설정이 없는 프로젝트에서는 아무것도 하지 않는다.

가상환경을 만들거나 의존성을 설치하지 않는다. 도구는 아래 순서로 찾는다.

1. 프로젝트의 `.venv`
2. git worktree 본체의 `.venv`
3. 활성화된 가상환경 (`VIRTUAL_ENV`, conda의 `CONDA_PREFIX`)
4. ruff만: `PATH` (ruff는 환경과 무관하게 동작한다. ty와 pytest는 프로젝트 의존성이 있는 환경이 필요해서 `PATH`에서 찾지 않는다)

설정은 있는데 도구를 못 찾으면 그 검사는 건너뛰고, 훅에서는 대화마다 한 번 알린다. `kiln status`로 어떤 검사가 어떤 도구로 도는지 확인한다.

| 조건 | 명령 | 범위 | 훅 | check |
|---|---|---|---|---|
| `[tool.ruff` | `ruff check` | 바뀐 `.py` 파일 | O | O |
| `[tool.ruff.format]` | `ruff format --check` | 바뀐 `.py` 파일 | O | O |
| `[tool.ty` | `ty check` | 프로젝트 전체 | O | O |
| `[tool.pytest` | `pytest -q` | 프로젝트 전체 | | O |
| `Cargo.toml` | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | 프로젝트 전체 | O | O |
| `Cargo.toml` | `cargo test` | 프로젝트 전체 | | O |

린트와 포맷은 새로 생기는 위반을 막는 것이 목적이라 바뀐 파일만 본다. 타입과 Rust 검사는 한 파일의 변경이 다른 파일을 깨뜨릴 수 있어 전체를 본다. 테스트는 시간이 걸려 훅에서는 돌리지 않는다.

### 주석과 검사 끄기 표시

프로젝트 설정에 아래가 있으면 새로 추가된 줄에서 주석, 파일 맨 위 설명문, 검사 끄기 표시를 막는다.

    # pyproject.toml
    [tool.kiln]
    comments = false

    # Cargo.toml
    [package.metadata.kiln]
    comments = false

- 새로 추가되거나 바뀐 줄만 본다. 원래 있던 주석은 건드리지 않는다.
- 파이썬은 줄 전체 주석, 줄 끝 주석, 파일 맨 위 설명문을 본다. 문자열 안의 `#`은 주석이 아니다. 함수와 클래스의 설명문은 막지 않는다.
- Rust는 `//`로 시작하는 줄을 본다. 문서 주석(`///`, `//!`)은 막지 않는다.
- 검사 끄기 표시를 막는다: `noqa`, `type: ignore`, `ty: ignore`, `pyright: ignore`, `mypy: ignore`, `ruff: noqa`, `pylint: disable`, Rust의 `#[allow(`, `#[expect(`. 꼭 필요한 예외는 설정 파일(`per-file-ignores`, `[lints]`)에 모은다.
- 통과하는 주석: 도구 지시(`pragma`, `fmt:`, `type:` 타입 주석, `coding`), 첫 줄의 `#!`.

### 저장소 밖 설정

남의 저장소처럼 설정 파일을 고치기 곤란한 곳은 `~/.config/kiln/config.toml`에서 켠다. 저장소 안 설정보다 우선하고, 경로가 여러 개 걸리면 가장 깊은 것을 쓴다.

```toml
[[project]]
path = "~/vllm"
comments = false
```

파이썬 주석 찾기는 Python 표준 `tokenize`, `ast`와 결과가 같은지 대조한다. vllm과 pytorch의 9,608개 파일에서 불일치 0건이었다.

    python3 scripts/compare_python_comments.py target/release/kiln <디렉터리>...

### 커밋 메시지

첫 줄은 `타입: 요약` (`feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `build`, `ci`, `style`, `revert`, 범위와 `!` 허용). 공동 작성자(`Co-Authored-By`) 줄은 막는다. git이 만드는 `Merge`, `Revert`, `fixup!` 등은 통과한다.

## 붙이는 법

pre-commit (`.pre-commit-config.yaml`):

```yaml
default_install_hook_types: [pre-commit, commit-msg]
repos:
  - repo: local
    hooks:
      - id: kiln
        name: kiln check
        entry: kiln check --staged
        language: system
        pass_filenames: false
      - id: kiln-commit-msg
        name: kiln commit-msg
        entry: kiln commit-msg
        language: system
        stages: [commit-msg]
```

CI (GitHub Actions):

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- run: curl -fsSL https://raw.githubusercontent.com/kihan-lee/kiln/main/scripts/install.sh | bash
- run: ~/.local/bin/kiln check --base origin/main
```

Claude Code (`~/.claude/settings.json`):

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "kiln hook start" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "kiln hook stop", "timeout": 600, "statusMessage": "kiln 검사 중" }] }
    ]
  }
}
```

### 훅의 동작

- 대화가 시작될 때 작업 폴더 상태를 저장하고, 답변이 끝날 때 그 뒤로 바뀐 파일과 줄만 검사한다. 대화 전부터 고치던 파일은 대화 시작 때 내용과 비교한다.
- 바뀐 파일이 없거나 마지막으로 통과한 상태와 같으면 건너뛴다.
- 통과하면 아무것도 출력하지 않는다. 모델 문맥에 들어가는 것이 없어 토큰을 쓰지 않는다.
- 실패하면 답변 종료를 막고 출력 끝 40줄을 모델에 돌려준다. 프로젝트마다 처음 실패한 명령에서 멈춘다.
- 한 대화에서 3번 연속 막은 뒤에는 더 막지 않고 사용자에게 알린다.
- 상태는 `~/.local/state/kiln`에 두고, 14일 지난 것은 대화가 시작될 때 지운다.

### 기록

훅이 검사를 실제로 돌릴 때마다 결과(통과, 막음, 포기와 걸린 검사, 지적 건수)를 `~/.local/state/kiln/log.jsonl`에 남긴다. `kiln stats`는 기간, 통과와 막음 횟수, 막은 뒤 같은 대화에서 통과까지 간 비율, 검사별 막은 횟수와 지적 건수를 보여 준다.

## 개발

    scripts/setup.sh     # 소스로 설치하고 이 저장소의 커밋 검사 켜기
    scripts/check.sh     # 포맷, 린트, 테스트

`v*` 태그를 올리면 릴리스 워크플로가 실행 파일을 빌드해 GitHub Release에 올린다.

## 라이선스

[MIT](LICENSE)
