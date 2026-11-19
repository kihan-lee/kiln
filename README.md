# dev-standard

이 장비에서 만드는 프로젝트의 표준 도구. 프로젝트 템플릿과 Claude Code 훅을 한 저장소에서 관리한다.

원칙: 지적을 받아 고친 것은 글(규칙 문서, 메모리)로 남기기 전에 검사로 옮긴다. 순서는 ruff 규칙이나 `banned-api`, 타입, 테스트이고, 기계로 못 잡는 것만 글로 적는다.

## 구성

| 경로 | 역할 |
|---|---|
| `template/` | 파이썬 프로젝트 템플릿 (uv, Hydra, pydantic, pytest, ruff, ty, pre-commit, gitleaks, Bruno) |
| `hook/` | Claude Code 훅 (Rust). 답변이 끝날 때 파일이 바뀌었으면 그 프로젝트의 검사를 돌린다 |
| `scripts/check.sh` | 이 저장소 자체의 검사 (훅 소스의 포맷과 린트, 템플릿 검증) |
| `scripts/check-template.sh` | 템플릿 검증 |
| `scripts/setup.sh` | 장비마다 한 번 실행. 훅 설치와 이 저장소의 커밋 검사 켜기 |
| `.pre-commit-config.yaml` | 이 저장소의 커밋 검사 (훅 소스의 포맷과 린트, 커밋 메시지 형식, 비밀값) |

## 새 프로젝트

    mkdir <프로젝트 이름> && cd <프로젝트 이름>
    uv tool run copier copy --trust --defaults ~/app/dev-standard .

템플릿이 바뀐 뒤 기존 프로젝트에 반영:

    uv tool run copier update --trust --defaults

태그(`v0.4.0` 등)를 올려야 `copier update`에 반영된다.

## 검사가 도는 시점

| 시점 | 내용 | 누가 |
|---|---|---|
| Claude의 답변이 끝날 때 (파일이 바뀌었을 때만) | lint, 포맷, 타입 | 훅 |
| 커밋할 때 | lint, 포맷, 테스트, 비밀값 검사 | pre-commit |
| 직접 실행, CI | lint, 포맷, 타입, 테스트 | 프로젝트의 `scripts/check.sh` |

## Claude Code 훅

프로젝트에 따로 넣을 것은 없다. 훅이 바뀐 파일에서 위로 올라가며 가장 가까운 `pyproject.toml`이나 `Cargo.toml`을 찾고, 설정된 도구만 돌린다.

| 조건 | 실행하는 명령 | 범위 |
|---|---|---|
| `pyproject.toml`에 `[tool.ruff`가 있고 ruff가 설치돼 있음 | `ruff check` | 이번에 바뀐 `.py` 파일만 |
| 위에 더해 `[tool.ruff.format]`이 있음 | `ruff format --check` | 이번에 바뀐 `.py` 파일만 |
| `pyproject.toml`에 `[tool.ty`가 있고 ty가 설치돼 있음 | `ty check` | 프로젝트 전체 |
| `Cargo.toml`이 있음 | `cargo fmt --check`, `cargo clippy -q -- -D warnings` | 프로젝트 전체 |

린트와 포맷은 새로 생기는 위반을 막는 것이 목적이라 바뀐 파일만 본다. 예전 위반은 따로 정리한다. 타입 검사와 Rust 검사는 한 파일의 변경이 다른 파일을 깨뜨릴 수 있어 전체를 본다.

훅은 가상환경을 만들거나 의존성을 설치하지 않는다. 도구는 그 프로젝트의 `.venv`에서 찾고, 없으면 같은 저장소의 원래 작업 폴더(git worktree의 본체)에 있는 `.venv`에서 찾는다. 둘 다 없으면 그 검사는 건너뛴다.

어떤 검사를 빼려면 그 프로젝트의 `pyproject.toml`에서 해당 설정을 넣지 않는다.

그 밖의 동작:

- 대화가 시작된 뒤 작업 폴더가 그대로면 건너뛴다.
- 마지막으로 통과한 상태와 같으면 건너뛴다.
- 통과하면 아무것도 출력하지 않는다. 모델 문맥에 들어가는 것이 없어 토큰을 쓰지 않는다.
- 실패하면 답변 종료를 막고 출력 끝 40줄을 모델에 돌려준다. 프로젝트마다 처음 실패한 명령에서 멈춘다.
- 한 대화에서 3번 연속 막은 뒤에는 더 막지 않고 사용자에게 알린다.

설치 (장비마다 한 번. 훅을 고친 뒤에도 같은 명령으로 다시 설치):

    scripts/setup.sh

훅 실행 파일을 `~/.cargo/bin`에 설치하고, 이 저장소의 커밋 검사(pre-commit)를 켠다.

`~/.claude/settings.json`:

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "~/.cargo/bin/dev-standard-hook start" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "~/.cargo/bin/dev-standard-hook stop", "timeout": 600, "statusMessage": "표준 검사 실행 중" }] }
    ]
  }
}
```

## 고친 뒤

커밋할 때 훅 소스의 포맷과 린트, 커밋 메시지 형식, 비밀값 검사가 자동으로 돈다. 템플릿 검증은 시간이 걸려 커밋 검사에 넣지 않았으니 직접 실행한다.

    scripts/check.sh

커밋한 뒤 실행한다. 템플릿 검증은 임시 프로젝트를 만들어 표준 검사가 바로 통과하는지, `copier update`가 프로젝트를 바꾸지 않는지 확인한다.
