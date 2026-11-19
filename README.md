# dev-standard

이 장비에서 만드는 파이썬 프로젝트의 표준 도구. 프로젝트 틀, 검사 진입점, Claude Code 세션 훅을 한 저장소에서 관리한다.

원칙: 지적을 받아 고친 것은 글(규칙 문서, 메모리)로 남기기 전에 검사로 옮긴다. 순서는 ruff 규칙이나 `banned-api`, 타입, 테스트이고, 기계로 못 잡는 것만 글로 적는다.

## 구성

| 경로 | 역할 |
|---|---|
| `template/` | 프로젝트 틀 (uv, Hydra, pydantic, pytest, ruff, pyright, pre-commit, gitleaks, Bruno) |
| `template/scripts/check.sh.jinja` | 표준 검사 진입점. `--quick`은 lint, 포맷, 타입만, 인자 없이 부르면 테스트까지 |
| `hook/` | Claude Code 훅 (Rust). 답변이 끝날 때 파일이 바뀌었으면 `scripts/check.sh --quick`을 돌린다 |
| `scripts/check.sh` | 이 저장소 자체의 표준 검사 (훅 소스의 포맷과 린트, 틀 검증) |
| `scripts/check-template.sh` | 틀 자체의 검증 |

## 새 프로젝트

    mkdir <프로젝트 이름> && cd <프로젝트 이름>
    uv tool run copier copy --trust --defaults ~/app/dev-standard .

틀이 바뀐 뒤 기존 프로젝트에 반영:

    uv tool run copier update --trust --defaults

태그(`v0.4.0` 등)를 올려야 `copier update`에 반영된다.

## 검사

| 단계 | 내용 | 언제 |
|---|---|---|
| `scripts/check.sh --quick` | lint, 포맷, 타입 | Claude Code 훅이 답변 끝마다 (파일이 바뀌었을 때만) |
| `scripts/check.sh` | 빠른 검사 + 테스트 | 직접 실행, CI |
| pre-commit | lint, 포맷, 테스트, 비밀값 검사 | 커밋할 때 |

빠른 검사에는 몇 초 안에 끝나는 것만 둔다. 프로젝트마다 검사를 더하려면 `scripts/check.sh`의 해당 단계에 줄을 추가한다.

## Claude Code 훅

설치 (고친 뒤에도 같은 명령으로 다시 설치):

    cargo install --path hook

`~/.claude/settings.json`에 아래를 넣으면 모든 프로젝트에 적용된다.

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

동작:

- 바뀐 파일에서 위로 올라가며 가장 가까운 `scripts/check.sh`를 찾아 `--quick`으로 실행한다. 없는 저장소에서는 아무 일도 하지 않는다.
- 세션이 작업 폴더를 바꾸지 않았으면 건너뛴다.
- 마지막으로 통과한 상태와 같으면 건너뛴다.
- 통과하면 아무것도 출력하지 않는다. 모델 문맥에 들어가는 것이 없어 토큰을 쓰지 않는다.
- 실패하면 종료를 막고 출력 끝 40줄을 모델에 돌려준다.
- 한 세션에서 3번 연속 막은 뒤에는 더 막지 않고 사용자에게 알린다.

## 틀을 고친 뒤

    scripts/check-template.sh [--pre-commit]

커밋한 뒤 실행한다. 임시 프로젝트를 만들어 표준 검사가 바로 통과하는지, `copier update`가 프로젝트를 바꾸지 않는지 확인한다.
