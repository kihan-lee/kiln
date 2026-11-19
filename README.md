# ai-python-project-template

uv, Hydra, pydantic, pytest, ruff, pre-commit, gitleaks, Bruno 파이썬 프로젝트 템플릿 (copier).

## 생성

    mkdir <프로젝트 이름> && cd <프로젝트 이름>
    uv tool run copier copy --trust --defaults git@github.com:kihan-lee/ai-python-project-template.git .

## 업데이트

    uv tool run copier update --trust --defaults

v0.3.0 이하로 만든 프로젝트는 처음 한 번만 이름을 넘긴다 (이후 answers 파일에 기록됨).
이때는 파일 전체가 충돌로 표시될 수 있다. 해결 후 커밋하면 이후 업데이트는 정상 동작한다.

    uv tool run copier update --trust --defaults -d project_name=<프로젝트 이름>

## 검증

    scripts/check-template.sh [--pre-commit]

커밋한 뒤 실행한다. 수정 후 최신 태그(`v0.3.1` 등)를 올려야 `copier update`에 반영된다.
