# kiln

에이전트가 쓴 코드를 굽는 가마. 새로 생긴 위반만 골라 막는 품질 검사 도구다. 같은 검사를 Claude Code 훅, 커밋(pre-commit), CI에서 돌린다.

원칙: 지적을 받아 고친 것은 글(규칙 문서, 메모리)로 남기기 전에 검사로 옮긴다. 순서는 린트 규칙, 타입, 테스트이고, 기계로 못 잡는 것만 글로 적는다.

## 설치

    cargo install --git https://github.com/kihan-lee/kiln --locked

`~/.cargo/bin/kiln`에 설치된다. 새 버전도 같은 명령으로 받는다. `~/.cargo/bin`이 `PATH`에 있어야 한다. rustup으로 Rust를 설치했으면 이미 들어 있다. Homebrew 같은 다른 방법으로 설치했으면 `~/.zshenv`(bash는 `~/.profile`)에 `export PATH="$HOME/.cargo/bin:$PATH"`를 넣는다. `~/.zshrc`는 대화형 셸만 읽으므로 쓰지 않는다. 훅은 세션이 시작될 때 kiln이 `PATH`에 없으면 실행 파일 위치를 모델에 알려 준다.

cargo가 없는 곳(CI 등)에서는 미리 빌드한 실행 파일을 받는다 (Linux x86_64, aarch64, macOS, `~/.local/bin/kiln`):

    curl -fsSL https://raw.githubusercontent.com/kihan-lee/kiln/main/scripts/install.sh | bash

## 명령

| 명령 | 쓰는 곳 | 검사 범위 |
|---|---|---|
| `kiln check --staged` | pre-commit | 커밋할 내용 |
| `kiln check --base <ref>` | CI | `<ref>`와 갈라진 뒤 바뀐 내용 |
| `kiln check --all` | 직접 실행 | 프로젝트 전체 (기존 주석과 검사 끄기 표시는 목록만 보여 주고 실패로 치지 않음) |
| `kiln commit-msg <file>` | commit-msg | 커밋 메시지 형식 |
| `kiln hook start`, `kiln hook track`, `kiln hook stop` | Claude Code 훅 | 대화가 시작된 뒤 이 대화가 바꾼 내용 |
| `kiln status` | 직접 실행 | 지금 위치에서 어떤 검사가 어떤 도구로 도는지 |
| `kiln stats` | 직접 실행 | 훅 기록 요약 |
| `kiln adopt strip [경로...]` | 직접 실행 | 기존 파이썬 주석과 검사 끄기 표시를 전부 지움 |
| `kiln adopt verify [--base <ref>]` | 직접 실행, CI | `<ref>` 뒤로 주석과 설명문만 바뀌었는지 |

## 검사 내용

바뀐 파일에서 위로 올라가며 가장 가까운 프로젝트 폴더를 찾고, 그 프로젝트에 설정된 도구만 돌린다. 프로젝트 폴더는 `pyproject.toml`, `Cargo.toml`, `kiln.toml`, `.yamllint`(`.yamllint.yaml`, `.yamllint.yml`), 또는 `project(`를 선언한 `CMakeLists.txt`가 있는 곳이다. `add_subdirectory`로 들어가는 하위 `CMakeLists.txt`는 `project(`가 없으므로 상위 프로젝트에 속한다. 템플릿이 아닌 프로젝트에서도 그대로 동작한다. 설정이 없는 프로젝트에서는 아무것도 하지 않는다.

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
| `[tool.pytest` | `pytest -q` (훅에서는 `-x` 추가) | 프로젝트 전체 | 설정 시 | O |
| `Cargo.toml` | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | 프로젝트 전체 | O | O |
| `Cargo.toml` | `cargo test` | 프로젝트 전체 | 설정 시 | O |
| `.clang-format` | `clang-format --dry-run --Werror` | 바뀐 CUDA, C, C++ 파일 | O | O |
| `.yamllint` | `yamllint --no-warnings` | 바뀐 yaml 파일 | O | O |
| 항상 | `shellcheck --severity=warning` | 바뀐 `.sh`, `.bash` 파일 | O | O |
| 항상 | `kubeconform -strict -ignore-missing-schemas` | 바뀐 매니페스트를 그린 결과 | O | O |
| 항상 | `kube-linter lint` | 바뀐 매니페스트를 그린 결과 | O | O |
| Kyverno 정책 파일 | `kyverno apply` | 바뀐 매니페스트를 그린 결과 (정책이 바뀌면 전체) | O | O |

린트와 포맷은 새로 생기는 위반을 막는 것이 목적이라 바뀐 파일만 본다. 타입과 Rust 검사는 한 파일의 변경이 다른 파일을 깨뜨릴 수 있어 전체를 본다.

전체를 보는 검사(ty, pytest, cargo)는 그 프로젝트의 코드나 설정 파일이 바뀌었을 때만 돈다. 문서만 바뀌면 건너뛴다. 지워진 파일도 바뀐 것으로 친다. `--all`은 항상 전부 돈다.

- 파이썬: `.py`, `.pyi`, `pyproject.toml`, `uv.lock`, `ty.toml`
- Rust: `.rs`, `Cargo.toml`, `Cargo.lock`, `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml`
- CUDA, C, C++ (`.cu`, `.cuh`, `.c`, `.cc`, `.cpp`, `.cxx`, `.h`, `.hpp`): 커널을 고치면 결과가 달라지므로 pytest와 cargo 검사를 돌린다. ty는 돌리지 않는다.

clang-format은 프로젝트 폴더에 `.clang-format`(또는 `_clang-format`)이 있을 때만 돌고, `PATH`에서 찾는다.

### Kubernetes

yamllint, shellcheck, kubeconform, kube-linter, helm, kubectl(또는 kustomize)은 `PATH`에서 찾는다. 클러스터에는 접속하지 않는다. kubeconform은 스키마를 처음 받을 때만 네트워크를 쓰고 `~/.cache/kiln/kubeconform`에 저장한다.

바뀐 파일마다 무엇을 그릴지 정한다.

- `Chart.yaml`이 있는 폴더 안의 파일: 그 차트를 기본 values로 `helm template`
- `kustomization.yaml`이 있는 폴더 안의 파일: 그 폴더를 `kubectl kustomize`(`kustomize`가 있으면 `kustomize build`). kustomization이 그 파일이나 하위 폴더 이름을 적고 있지 않으면 속하지 않은 것으로 보고 위로 더 올라간다
- 어디에도 속하지 않는 yaml 중 `apiVersion:`과 `kind:`가 있는 파일: 그 파일 그대로

그린 결과를 kubeconform에 넘긴다. 그리기 자체가 실패해도 kubeconform 검사의 실패로 보고한다. 같은 결과를 kube-linter에도 넘긴다. 프로젝트 폴더에 `.kube-linter.yaml`이 있으면 그 설정을 쓰고, 없으면 kube-linter 기본 규칙에서 `no-read-only-root-fs`와 `run-as-non-root`를 뺀다(보안 강화 규칙이라 일반 워크로드에서 지적이 너무 많다). 어떤 규칙을 쓸지는 `.kube-linter.yaml`에서 정하고, 특정 객체의 예외는 `ignore-check.kube-linter.io/<규칙>: "이유"` annotation으로 남긴다. 이 annotation은 검사 끄기 표시로 본다(아래). 프로젝트 안에 `apiVersion: kyverno.io/...` 또는 `policies.kyverno.io/...`인 yaml이 있으면 그 정책 전부로 그린 결과를 `kyverno apply`에 넘긴다. Audit 정책의 위반도 막는다(kiln은 지금 바꾸는 것만 보므로 새 위반은 막는 것이 맞다). 정책 파일이 바뀌면 바뀐 정책이 기존 매니페스트를 깨뜨릴 수 있어 프로젝트의 모든 단위를 검사한다. 단위마다 따로 돌리므로 단위가 많으면 느리다. 클러스터의 다른 자원을 조회하는 정책은 오프라인에서 판단할 수 없다. kyverno CLI는 클러스터의 Kyverno와 같은 버전을 쓴다(1.20은 `kyverno.io/v1 ClusterPolicy`를 읽지 않는다). values 파일을 여러 개 쓰는 차트처럼 저장소마다 다른 규칙은 직접 정한 검사로 붙인다.

### 훅에서 테스트

테스트는 시간이 걸려 훅에서는 기본으로 돌리지 않는다. 빠른 테스트를 가진 프로젝트는 켤 수 있다. 훅에서는 첫 실패에서 멈춰(`pytest -x`) 바로 돌려준다.

    # pyproject.toml
    [tool.kiln]
    tests = true

    # Cargo.toml
    [package.metadata.kiln]
    tests = true

Claude Code 훅의 `timeout`(위 예시는 600초)보다 오래 걸리는 테스트는 켜지 않는다.

### 직접 정한 검사

형식 검증(TLA+, Lean), E2E 테스트처럼 kiln이 모르는 검증 도구도 같은 흐름(실패하면 막고 출력 끝부분을 돌려줌)에 올릴 수 있다. kiln은 명령을 대신 돌려줄 뿐 도구를 설치하거나 설정하지 않는다.

```toml
# pyproject.toml (Cargo.toml에서는 [[package.metadata.kiln.check]])
[[tool.kiln.check]]
name = "tla"
run = "tlc specs/Notify.tla"
hook = true
exts = ["tla", "py"]
```

| 키 | 뜻 |
|---|---|
| `name` | 결과와 `kiln stats`에 나오는 이름 |
| `run` | 프로젝트 폴더에서 `sh -c`로 실행할 명령. 종료 코드가 0이 아니면 실패 |
| `hook` | `true`면 훅에서도 돌린다. 기본은 check에서만 |
| `exts` | 이 확장자의 파일이 바뀔 때만 돌린다. 없으면 프로젝트 안의 파일이 하나라도 바뀌면 돈다. `--all`에서는 항상 돈다 |

내장 검사가 모두 통과한 뒤 적은 순서대로 돌고, 처음 실패한 것에서 멈춘다. 저장소 설정 파일에 적은 명령을 그대로 실행하므로, 믿을 수 없는 저장소에서는 `kiln check`를 돌리기 전에 이 설정을 확인한다.

### kiln.toml

`pyproject.toml`이나 `Cargo.toml`이 없는 프로젝트(CMake로 빌드하는 CUDA 등)는 프로젝트 폴더의 `kiln.toml`에 설정한다. 키는 `[tool.kiln]`과 같고 맨 위에 바로 쓴다. 같은 키가 여러 곳에 있으면 `kiln.toml`이 먼저다. 모르는 키는 오류로 알린다.

```toml
comments = false
docs = ["docs", "README.md"]

[[check]]
name = "ctest"
run = "cmake --build build && ctest --test-dir build --output-on-failure"
exts = ["cu", "cuh", "cpp", "h"]
```

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
- CUDA, C, C++는 `//` 주석과 `/* */` 주석의 각 줄을 본다. 문자열, raw 문자열(`R"(...)"`), 문자 리터럴 안은 주석이 아니고, 숫자 구분자(`1'000`)도 처리한다. 문서 주석(`///`, `//!`, `/** */`, `/*! */`)은 막지 않는다.
- 검사 끄기 표시를 막는다: `noqa`, `type: ignore`, `ty: ignore`, `pyright: ignore`, `mypy: ignore`, `ruff: noqa`, `pylint: disable`, Rust의 `#[allow(`, `#[expect(`, C 계열의 `NOLINT`, `#pragma nv_diag_suppress`, `#pragma diag_suppress`, `#pragma GCC diagnostic ignored`, `#pragma clang diagnostic ignored`, `#pragma warning(disable`, yaml의 `ignore-check.kube-linter.io/` annotation. yaml은 이 annotation만 보고 주석은 보지 않는다. 꼭 필요한 예외는 설정 파일(`per-file-ignores`, `[lints]`)에 모은다.
- 통과하는 주석: 도구 지시(`pragma`, `fmt:`, `type:` 타입 주석, `coding`, `clang-format`), 첫 줄의 `#!`.

### 기존 주석 정리 (adopt)

kiln을 쓰지 않던 저장소에서 `comments = false`를 켜기 전에 기존 주석을 한 번에 지운다. 주석은 변명이 담기기 쉬우므로 옮길 곳을 따지지 않고 지운다. 지켜야 할 제약이었다면 테스트, assert, 타입으로 옮긴다. 지운 원문은 git 기록에 남는다. 지금은 파이썬만 지원한다.

    kiln adopt strip            # 지금 폴더 아래 git이 추적하는 .py 파일
    kiln adopt strip src tests  # 경로 지정
    kiln adopt verify           # HEAD 대비 주석과 설명문만 바뀌었는지 확인

- `strip`은 주석 검사가 막는 것(주석, 검사 끄기 표시, 파일 맨 위 설명문)을 지운다. 줄 전체가 주석이면 줄을 지우고, 줄 끝 주석은 주석만 지운다. 통과하는 주석(`#!`, 인코딩 선언, `# type:`, `# fmt:`, `# pragma`)과 함수, 클래스 설명문은 남긴다.
- 파일마다 지우기 전과 뒤의 AST(실행에 영향을 주는 것만 남긴 구문 트리)를 비교해 같을 때만 쓴다. 다르거나 구문 분석에 실패하면 그 파일은 건드리지 않고 목록으로 알린다.
- 파일 맨 위 설명문은 doctest(`>>>`)가 있거나 파일이 `__doc__`를 쓰면 남긴다. 지우면 동작이나 테스트가 바뀌기 때문이다.
- 커밋하지 않은 변경이 있는 파일이 대상에 있으면 실행하지 않는다. 주석 삭제만 담은 커밋으로 남겨야 리뷰와 되돌리기가 쉽다.
- 검사 끄기 표시를 지우면 ruff, ty가 실패할 수 있다. 원인을 고치거나 설정(`per-file-ignores` 등)으로 옮긴다.
- `verify`는 `<ref>`(기본 `HEAD`)와 작업 폴더의 바뀐 `.py` 파일을 AST로 비교해, 주석과 설명문 말고 코드가 바뀐 파일이 있으면 실패한다. 손으로 정리한 커밋이나 정리 중 섞여 들어간 수정을 잡는다.
- 파이썬은 프로젝트 `.venv`, 활성 가상환경, `PATH`의 `python3` 순서로 찾는다. 프로젝트가 쓰는 문법을 읽을 수 있는 버전이어야 한다.

Python 3.15 표준 라이브러리 2,062개 파일에서 1,752개 파일, 주석 103,772개를 지웠고 `verify`가 통과했다. 일부러 문법 오류를 넣은 테스트 파일 3개와, 주석 찾기가 3.12부터 바뀐 f-string 문법(PEP 701, f-string 안의 주석과 따옴표 재사용)을 잘못 읽은 파일 1개는 AST 비교에서 걸려 그대로 남았다.

### 새 문서 파일과 규칙 파일

프로젝트 설정에 `docs`(새 문서 파일을 둘 수 있는 경로)를 적으면 켜진다.

    # pyproject.toml (Cargo.toml에서는 [package.metadata.kiln])
    [tool.kiln]
    docs = ["docs", "README.md", "CHANGELOG.md"]

- 허용 경로 밖에 새로 생긴 `.md` 파일을 막는다. 에이전트가 요청받지 않은 작업 메모, 요약 문서를 쌓는 것을 막기 위해서다. 이미 있던 문서를 고치는 것은 막지 않는다.
- 주석은 Python, Rust, C/C++, YAML, 셸, SQL에서 찾는다. YAML 블록 문자열(`|`, `>`) 안과 셸 heredoc 안의 `#`은 내용으로 보고 건너뛴다. `# shellcheck disable`, `# yamllint disable`은 검사 끄기 표시로 막는다.
- 규칙 파일(`CLAUDE.md`, `CLAUDE.local.md`, `AGENTS.md`, `.claude/` 아래의 `.md`)은 막지 않는다. 대신 훅에서 바뀐 것을 한 번 알려, 그 내용을 검사로 옮길 수 있는지 먼저 확인하게 한다. 같은 내용으로는 다시 막지 않는다.
- 규칙 파일 알림은 훅에서만, 새 문서 파일 검사는 훅, 커밋, CI 모두에서 한다.
- 문서 내용은 보지 않는다. 내용이 정당화인지는 기계로 판단할 수 없다.

### 저장소 밖 설정

남의 저장소처럼 설정 파일을 고치기 곤란한 곳은 `~/.config/kiln/config.toml`에서 켠다. 저장소 안 설정보다 우선하고, 경로가 여러 개 걸리면 그 설정을 적은 것 중 가장 깊은 것을 쓴다.

```toml
[[project]]
path = "~/vllm"
comments = false
tests = true
docs = ["docs"]
```

파이썬 주석 찾기는 Python 표준 `tokenize`, `ast`와 결과가 같은지 대조한다. vllm과 pytorch의 9,608개 파일에서 불일치 0건이었다.

    python3 scripts/compare_python_comments.py target/release/kiln <디렉터리>...

CUDA, C, C++ 주석 찾기는 clang 렉서(`-dump-raw-tokens`)와 대조한다. cutlass, llm.c, llama.cpp의 4,145개 파일에서 불일치 0건이었다.

    python3 scripts/compare_c_comments.py target/release/kiln <디렉터리>...

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
    "PreToolUse": [
      { "hooks": [{ "type": "command", "command": "kiln hook track" }] }
    ],
    "PostToolUse": [
      { "hooks": [{ "type": "command", "command": "kiln hook track" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "kiln hook stop", "timeout": 600, "statusMessage": "kiln 검사 중" }] }
    ]
  }
}
```

### 훅의 동작

- 대화가 시작될 때 작업 폴더 상태를 저장하고, 답변이 끝날 때 그 뒤로 바뀐 파일과 줄만 검사한다. 대화 전부터 고치던 파일은 대화 시작 때 내용과 비교한다.
- `kiln hook track`(PreToolUse, PostToolUse)을 켜면 이 대화가 바꾼 파일만 검사한다. 도구를 부르기 직전과 직후에 바뀐 파일, 편집 도구가 지정한 파일을 이 대화의 것으로 적어 둔다. 같은 작업 폴더에서 다른 대화나 사람이 고치는 파일은 검사하지 않으므로, 여러 대화가 한 저장소를 같이 써도 서로 막지 않는다. 같은 시각에 다른 대화가 고친 파일은 이 대화의 것으로 잡힐 수 있다. 다른 대화가 편집 도구로 고쳤거나 명령, 스크립트에 경로를 적은 파일은 그 대화의 것으로 보고 뺀다. 경로를 적지 않고(예: 와일드카드) 고친 파일은 구분하지 못한다. 대화마다 `git worktree`로 작업 폴더를 나누면 이런 일이 없다.
- `track`을 켜지 않으면 대화가 시작된 뒤 바뀐 파일을 모두 검사한다.
- 저장소는 대화를 연 폴더가 아니라 고친 파일로 찾는다. 상위 폴더(예: `~/Desktop`)에서 연 대화도, 한 대화에서 고친 저장소 여러 개도 각각 검사한다. 처음 건드린 저장소는 그 순간의 작업 폴더 상태를 기준으로 삼는다. 셸 명령이나 스크립트로 고친 저장소는 명령에 적힌 경로로 찾는다. 경로를 적지 않고 고친 저장소는 찾지 못한다.
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
