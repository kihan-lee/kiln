# dev-standard

이 장비에서 만드는 프로젝트의 표준 도구. 프로젝트 틀과 Claude Code 훅을 한 저장소에서 관리한다.

원칙: 지적을 받아 고친 것은 글(규칙 문서, 메모리)로 남기기 전에 검사로 옮긴다. 순서는 ruff 규칙이나 `banned-api`, 타입, 테스트이고, 기계로 못 잡는 것만 글로 적는다.

## 구성

| 경로 | 역할 |
|---|---|
| `template/` | 파이썬 프로젝트 틀 (uv, Hydra, pydantic, pytest, ruff, pyright, pre-commit, gitleaks, Bruno) |
| `hook/` | Claude Code 훅 (Rust). 답변이 끝날 때 파일이 바뀌었으면 그 프로젝트의 검사를 돌린다 |
| `scripts/check.sh` | 이 저장소 자체의 검사 (훅 소스의 포맷과 린트, 틀 검증) |
| `scripts/check-template.sh` | 틀 검증 |

## 새 프로젝트

    mkdir <프로젝트 이름> && cd <프로젝트 이름>
    uv tool run copier copy --trust --defaults ~/app/dev-standard .

틀이 바뀐 뒤 기존 프로젝트에 반영:

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

| 조건 | 실행하는 명령 |
|---|---|
| `uv.lock`이 있고 `pyproject.toml`에 `[tool.ruff`가 있음 | `uv run ruff check .` |
| `uv.lock`이 있고 `[tool.ruff.format]`이 있음 | `uv run ruff format --check .` |
| `uv.lock`이 있고 `[tool.pyright]`가 있음 | `uv run pyright` |
| `Cargo.toml`이 있음 | `cargo fmt --check`, `cargo clippy -q -- -D warnings` |

어떤 검사를 빼려면 그 프로젝트의 `pyproject.toml`에서 해당 설정을 넣지 않는다.

그 밖의 동작:

- 대화가 시작된 뒤 작업 폴더가 그대로면 건너뛴다.
- 마지막으로 통과한 상태와 같으면 건너뛴다.
- 통과하면 아무것도 출력하지 않는다. 모델 문맥에 들어가는 것이 없어 토큰을 쓰지 않는다.
- 실패하면 답변 종료를 막고 출력 끝 40줄을 모델에 돌려준다. 프로젝트마다 처음 실패한 명령에서 멈춘다.
- 한 대화에서 3번 연속 막은 뒤에는 더 막지 않고 사용자에게 알린다.

설치 (고친 뒤에도 같은 명령으로 다시 설치):

    cargo install --path hook

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

    scripts/check.sh

커밋한 뒤 실행한다. 틀 검증은 임시 프로젝트를 만들어 표준 검사가 바로 통과하는지, `copier update`가 프로젝트를 바꾸지 않는지 확인한다.
