# dev-standard

이 장비에서 만드는 파이썬 프로젝트의 표준 도구. 프로젝트 틀, 검사 진입점, Claude Code 세션 훅을 한 저장소에서 관리한다.

원칙: 지적을 받아 고친 것은 글(규칙 문서, 메모리)로 남기기 전에 검사로 옮긴다. 순서는 ruff 규칙이나 `banned-api`, 타입, 테스트이고, 기계로 못 잡는 것만 글로 적는다.

## 구성

| 경로 | 역할 |
|---|---|
| `template/` | 프로젝트 틀 (uv, Hydra, pydantic, pytest, ruff, pyright, pre-commit, gitleaks, Bruno) |
| `template/scripts/check.sh` | 표준 검사 진입점. lint, 포맷, 타입, 테스트를 확인만 한다 |
| `claude/stop_check.py` | Claude Code 훅. 세션이 파일을 바꿨으면 끝나기 전에 `scripts/check.sh`를 돌린다 |
| `scripts/check-template.sh` | 틀 자체의 검증 |

## 새 프로젝트

    mkdir <프로젝트 이름> && cd <프로젝트 이름>
    uv tool run copier copy --trust --defaults ~/app/dev-standard .

틀이 바뀐 뒤 기존 프로젝트에 반영:

    uv tool run copier update --trust --defaults

태그(`v0.4.0` 등)를 올려야 `copier update`에 반영된다.

## 검사

프로젝트 안에서 `scripts/check.sh`. 프로젝트마다 검사를 더하려면 그 파일에 줄을 추가한다.

## Claude Code 훅

`~/.claude/settings.json`에 아래를 넣으면 모든 프로젝트에 적용된다.

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "python3 ~/app/dev-standard/claude/stop_check.py start" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "python3 ~/app/dev-standard/claude/stop_check.py stop", "timeout": 600, "statusMessage": "표준 검사 실행 중" }] }
    ]
  }
}
```

동작:

- 바뀐 파일에서 위로 올라가며 가장 가까운 `scripts/check.sh`를 찾아 실행한다. 없는 저장소에서는 아무 일도 하지 않는다.
- 세션이 작업 폴더를 바꾸지 않았으면 건너뛴다.
- 마지막으로 통과한 상태와 같으면 건너뛴다.
- 실패하면 종료를 막고 출력 끝 60줄을 모델에 돌려준다.
- 한 세션에서 3번 연속 막은 뒤에는 더 막지 않고 사용자에게 알린다.
- `DEV_STANDARD_SKIP=1`이면 끈다.

## 틀을 고친 뒤

    scripts/check-template.sh [--pre-commit]

커밋한 뒤 실행한다. 임시 프로젝트를 만들어 표준 검사가 바로 통과하는지, `copier update`가 프로젝트를 바꾸지 않는지 확인한다.
