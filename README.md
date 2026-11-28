# kiln

[한국어](README.ko.md)

A kiln for code written by agents. kiln is a quality gate that only blocks new violations, and it runs the same checks from a Claude Code hook, a pre-commit hook, and CI.

The rule behind it: when you get corrected and fix something, turn the fix into a check before you write it down in a rules file or memory. Try a lint rule first, then a type, then a test. Only write down what a machine can't catch.

## Install

    cargo install --git https://github.com/kihan-lee/kiln --locked

This puts the binary at `~/.cargo/bin/kiln`. Run the same command to upgrade. `~/.cargo/bin` has to be on your `PATH`. If you installed Rust with rustup, it already is. If you got Rust some other way (Homebrew, for example), add `export PATH="$HOME/.cargo/bin:$PATH"` to `~/.zshenv` (`~/.profile` for bash). Don't use `~/.zshrc`, since only interactive shells read it. If kiln isn't on `PATH` when a session starts, the hook tells the model where the binary is.

Where there's no cargo (CI and so on), download a prebuilt binary instead (Linux x86_64, aarch64, and macOS; installs to `~/.local/bin/kiln`):

    curl -fsSL https://raw.githubusercontent.com/kihan-lee/kiln/main/scripts/install.sh | bash

## Commands

| Command | Used from | What it checks |
|---|---|---|
| `kiln check --staged` | pre-commit | What you're about to commit |
| `kiln check --base <ref>` | CI | Changes since the branch split from `<ref>` |
| `kiln check --all` | by hand | The whole project (existing comments and suppressions are listed but don't fail the run) |
| `kiln commit-msg <file>` | commit-msg | Commit message format |
| `kiln hook start`, `kiln hook track`, `kiln hook stop` | Claude Code hooks | What this session changed since it started |
| `kiln status` | by hand | Which checks run here, and with which tools |
| `kiln stats` | by hand | Summary of the hook log |
| `kiln clean [path...]` | by hand | Removes all existing comments and suppressions |
| `kiln clean --verify [--base <ref>]` | by hand, CI | Whether only comments and docstrings changed since `<ref>` |

## What gets checked

For each changed file, kiln walks up to the nearest project root and runs only the tools that project is set up for. A project root is a directory with `pyproject.toml`, `Cargo.toml`, `kiln.toml`, `.yamllint` (or `.yamllint.yaml`, `.yamllint.yml`), or a `CMakeLists.txt` that calls `project(`. A nested `CMakeLists.txt` pulled in with `add_subdirectory` has no `project(`, so it belongs to the parent project. None of this depends on a template; it works on any project. If a project has no config, kiln does nothing there.

kiln doesn't create virtualenvs or install dependencies. It looks for tools in this order:

1. The project's `.venv`
2. The `.venv` of the main git worktree
3. The active environment (`VIRTUAL_ENV`, or conda's `CONDA_PREFIX`)
4. ruff only: `PATH` (ruff works the same in any environment; ty and pytest need one with the project's dependencies, so kiln doesn't look for them on `PATH`)

If a check is configured but its tool can't be found, kiln skips it, and the hook says so once per session. Run `kiln status` to see which checks run with which tools.

| When | Command | Scope | Hook | check |
|---|---|---|---|---|
| `[tool.ruff` | `ruff check` | Changed `.py` files | O | O |
| `[tool.ruff.format]` | `ruff format --check` | Changed `.py` files | O | O |
| `[tool.ty` | `ty check` | Whole project | O | O |
| `[tool.pytest` | `pytest -q` (plus `-x` in the hook) | Whole project | opt-in | O |
| `Cargo.toml` | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | Whole project | O | O |
| `Cargo.toml` | `cargo test` | Whole project | opt-in | O |
| `.clang-format` | `clang-format --dry-run --Werror` | Changed CUDA, C, C++ files | O | O |
| `.yamllint` | `yamllint --no-warnings` | Changed YAML files | O | O |
| always | `shellcheck --severity=warning` | Changed `.sh`, `.bash` files | O | O |
| always | `kubeconform -strict -ignore-missing-schemas` | Rendered output of changed manifests | O | O |
| always | `kube-linter lint` | Rendered output of changed manifests | O | O |
| Kyverno policy files | `kyverno apply` | Rendered output of changed manifests (everything, if a policy changed) | O | O |

Lint and format checks exist to stop new violations, so they only look at changed files. Type checks and the Rust checks look at the whole project, because a change in one file can break another.

Whole-project checks (ty, pytest, cargo) only run when that project's code or config changed. A docs-only change skips them. Deleted files count as changes. `--all` always runs everything.

- Python: `.py`, `.pyi`, `pyproject.toml`, `uv.lock`, `ty.toml`
- Rust: `.rs`, `Cargo.toml`, `Cargo.lock`, `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml`
- CUDA, C, C++ (`.cu`, `.cuh`, `.c`, `.cc`, `.cpp`, `.cxx`, `.h`, `.hpp`): changing a kernel changes results, so these trigger pytest and the cargo checks. They don't trigger ty.

clang-format only runs when the project root has a `.clang-format` (or `_clang-format`), and kiln finds it on `PATH`.

### Kubernetes

yamllint, shellcheck, kubeconform, kube-linter, helm, and kubectl (or kustomize) are found on `PATH`. kiln never talks to a cluster. kubeconform only hits the network the first time it fetches a schema, and caches it in `~/.cache/kiln/kubeconform`.

For each changed file, kiln decides what to render:

- A file inside a directory with `Chart.yaml`: `helm template` on that chart with its default values
- A file inside a directory with `kustomization.yaml`: `kubectl kustomize` on that directory (`kustomize build` if `kustomize` is installed). If the kustomization doesn't list the file or its subdirectory, the file isn't part of it, and kiln keeps walking up
- Any other YAML file with `apiVersion:` and `kind:`: the file as is

The rendered output goes to kubeconform. If rendering itself fails, that's reported as a kubeconform failure. The same output goes to kube-linter. If the project root has a `.kube-linter.yaml`, kiln uses it. Otherwise it uses kube-linter's defaults minus `no-read-only-root-fs` and `run-as-non-root` (those are hardening rules and flag too much on ordinary workloads). Pick your rules in `.kube-linter.yaml`, and record exceptions for a specific object with an `ignore-check.kube-linter.io/<check>: "reason"` annotation. kiln treats that annotation as a suppression (see below).

If the project contains YAML with `apiVersion: kyverno.io/...` or `policies.kyverno.io/...`, kiln runs `kyverno apply` with all of those policies against the rendered output. Violations of Audit policies are blocked too: kiln only looks at what you're changing now, so blocking new violations is the right call. When a policy file changes, it can break manifests that were already there, so kiln checks every unit in the project. Each unit is run separately, which gets slow when there are many. Policies that look up other resources in the cluster can't be evaluated offline. Use a kyverno CLI that matches the Kyverno version on your cluster (1.20 doesn't read `kyverno.io/v1 ClusterPolicy`). Anything repo-specific, like a chart that uses several values files, belongs in a custom check.

### Tests in the hook

Tests take time, so the hook doesn't run them by default. Projects with fast tests can turn them on. In the hook, tests stop at the first failure (`pytest -x`) so you get the result back quickly.

    # pyproject.toml
    [tool.kiln]
    tests = true

    # Cargo.toml
    [package.metadata.kiln]
    tests = true

Don't turn this on for test suites that take longer than the Claude Code hook `timeout` (600 seconds in the example below).

### Custom checks

Verification tools kiln doesn't know about, like formal methods (TLA+, Lean) or E2E tests, can go through the same flow: block on failure and hand back the tail of the output. kiln just runs the command. It doesn't install or configure anything.

```toml
# pyproject.toml ([[package.metadata.kiln.check]] in Cargo.toml)
[[tool.kiln.check]]
name = "tla"
run = "tlc specs/Notify.tla"
hook = true
exts = ["tla", "py"]
```

| Key | Meaning |
|---|---|
| `name` | Name shown in results and in `kiln stats` |
| `run` | Command run with `sh -c` from the project root. A non-zero exit code is a failure |
| `hook` | If `true`, also runs in the hook. By default it only runs in `check` |
| `exts` | Only run when a file with one of these extensions changes. Without it, any change in the project triggers it. `--all` always runs it |

Custom checks run in the order listed, after all built-in checks pass, and stop at the first failure. They run whatever command is in the repo's config, so check that config before running `kiln check` in a repo you don't trust.

### kiln.toml

Projects without `pyproject.toml` or `Cargo.toml` (CUDA built with CMake, say) put their config in `kiln.toml` at the project root. The keys are the same as `[tool.kiln]`, written at the top level. If a key is set in more than one place, `kiln.toml` wins. Unknown keys are an error.

```toml
comments = false
docs = ["docs", "README.md"]

[[check]]
name = "ctest"
run = "cmake --build build && ctest --test-dir build --output-on-failure"
exts = ["cu", "cuh", "cpp", "h"]
```

### Comments and suppressions

With this in your project config, kiln blocks comments, module docstrings, and suppressions on newly added lines.

    # pyproject.toml
    [tool.kiln]
    comments = false

    # Cargo.toml
    [package.metadata.kiln]
    comments = false

- Only new or changed lines are checked. Existing comments are left alone.
- Python: full-line comments, trailing comments, and the module docstring. A `#` inside a string isn't a comment. Function and class docstrings are allowed.
- Rust: lines starting with `//`. Doc comments (`///`, `//!`) are allowed.
- CUDA, C, C++: `//` comments and every line of a `/* */` comment. Strings, raw strings (`R"(...)"`), and character literals aren't comments, and digit separators (`1'000`) are handled. Doc comments (`///`, `//!`, `/** */`, `/*! */`) are allowed.
- Suppressions are blocked: `noqa`, `type: ignore`, `ty: ignore`, `pyright: ignore`, `mypy: ignore`, `ruff: noqa`, `pylint: disable`, Rust's `#[allow(` and `#[expect(`, `NOLINT` in C-family code, `#pragma nv_diag_suppress`, `#pragma diag_suppress`, `#pragma GCC diagnostic ignored`, `#pragma clang diagnostic ignored`, `#pragma warning(disable`, and the `ignore-check.kube-linter.io/` annotation in YAML. For YAML, kiln only looks at that annotation, not at comments. Exceptions you really need go in config (`per-file-ignores`, `[lints]`).
- Comments that pass: tool directives (`pragma`, `fmt:`, `type:` type comments, `coding`, `clang-format`) and a `#!` on the first line.

### Cleaning up existing comments (clean)

Before turning on `comments = false` in a repo that wasn't using kiln, remove the existing comments in one go. Comments tend to collect excuses, so kiln deletes them without trying to find them a new home. If a comment described a constraint that matters, move it into a test, an assert, or a type. The deleted text stays in git history.

    kiln clean            # files tracked by git under the current directory
    kiln clean src tests  # specific paths
    kiln clean --verify   # check that only comments and docstrings changed since HEAD

- Removes comments, and suppressions written as comments, in every language the comment check covers (Python, Rust, CUDA, C, C++, YAML, shell, SQL). A full-line comment loses its line. A trailing comment loses just the comment. Comments that pass (`#!`, encoding declarations, `# type:`, `# fmt:`, `# pragma`, `# shellcheck source=`, and so on) and doc comments (`///`, `/** */`, function and class docstrings) are kept.
- A multi-line block comment in C-family code is handled as one piece. If it can't be removed cleanly, for example because it ends on a line with code, it's kept whole. For Rust, kiln uses a real Rust tokenizer (`proc-macro2`) to find comments between tokens, so it also removes comments in the middle of a line and nested block comments.
- Suppressions that are code (Rust `#[allow(`, C `#pragma ... diagnostic ignored`, the kube-linter `ignore-check` annotation) would change the code if removed, so kiln keeps them and lists them. Fix the cause or move them into config.
- For each file, kiln checks that the code before and after is the same, using a tool that has nothing to do with kiln's own comment finder, and only writes the file if it is. If the code differs or can't be checked, the file is left untouched and listed.

  | Language | How it's checked | Tool needed |
  |---|---|---|
  | Python | `ast` syntax tree | python3 (project `.venv`, then active environment, then `PATH`) |
  | Rust | `proc-macro2` token stream | none (built into kiln) |
  | CUDA, C, C++ | `clang -dump-raw-tokens` token stream | clang |
  | YAML | Parsed data | none (built into kiln) |
  | Helm templates | Chart rendered with `helm template` | helm |
  | Shell | `shfmt --to-json` syntax tree with positions and comments stripped | shfmt (`brew install shfmt`) |
  | SQL | Skipped, no tool to check with | |

- A Python module docstring is kept if it has doctests (`>>>`) or the file uses `__doc__`. Docstrings are values in the syntax tree, so the comparison ignores them, but in these two cases removing one would change behavior or tests.
- kiln refuses to run if any target file has uncommitted changes. A commit that only removes comments is easier to review and to revert.
- Removing suppressions can make lint fail. Fix the cause or move them into config (`per-file-ignores` and so on).
- `--verify` compares the changed files between `<ref>` (default `HEAD`) and the working tree the same way, and fails if any file changed in more than comments and docstrings. A file that can't be checked also counts as a failure. This catches cleanup commits done by hand, and real edits that slipped in during a cleanup.

Results (all passed `--verify` after cleanup):

| Target | Files cleaned | Comments removed | Files left |
|---|---|---|---|
| Python 3.15 standard library, 2,062 files | 1,752 | 103,772 | 4: three tests with deliberate syntax errors, and one where the comment finder misread the f-string syntax changed in 3.12 (PEP 701) |
| macOS SDK C headers, 457 files | 457 | 19,094 | 0 |
| 5 Rust crates (syn, serde, serde_json, proc-macro2, yaml-rust2) | 98 | 2,231 | 0 |
| A Kubernetes repo (270 YAML, 29 shell, 1 Helm chart) | 260 | 2,220 | 2 SQL |

### New docs and rules files

Turn this on by setting `docs` (the paths where new docs are allowed) in your project config.

    # pyproject.toml ([package.metadata.kiln] in Cargo.toml)
    [tool.kiln]
    docs = ["docs", "README.md", "CHANGELOG.md"]

- New `.md` files outside the allowed paths are blocked. This stops agents from piling up work notes and summaries nobody asked for. Editing docs that already exist is fine.
- Comments are detected in Python, Rust, C/C++, YAML, shell, and SQL. A `#` inside a YAML block scalar (`|`, `>`) or a shell heredoc is content, so it's skipped. `# shellcheck disable` and `# yamllint disable` are blocked as suppressions.
- Rules files (`CLAUDE.md`, `CLAUDE.local.md`, `AGENTS.md`, and `.md` files under `.claude/`) aren't blocked. Instead, the hook points out the change once, so you first ask whether it could be a check instead. The same content isn't flagged twice.
- The rules-file notice only happens in the hook. The new-docs check runs in the hook, at commit, and in CI.
- kiln doesn't read what the docs say. Whether something is a justification isn't a question a machine can answer.

### Config outside the repo

For repos where editing the config is awkward (someone else's repo, say), turn things on in `~/.config/kiln/config.toml`. This takes precedence over the repo's own config. If several paths match, the deepest one that sets the key wins.

```toml
[[project]]
path = "~/vllm"
comments = false
tests = true
docs = ["docs"]
```

The Python comment finder is cross-checked against Python's own `tokenize` and `ast`. Across 9,608 files from vllm and pytorch there were zero mismatches.

    python3 scripts/compare_python_comments.py target/release/kiln <dir>...

The CUDA, C, and C++ comment finder is cross-checked against the clang lexer (`-dump-raw-tokens`). Across 4,145 files from cutlass, llm.c, and llama.cpp there were zero mismatches.

    python3 scripts/compare_c_comments.py target/release/kiln <dir>...

### Commit messages

The first line is `type: summary` (`feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `build`, `ci`, `style`, `revert`; scopes and `!` are allowed). `Co-Authored-By` lines are blocked. Messages git generates itself (`Merge`, `Revert`, `fixup!`, and so on) pass.

## Setup

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
      { "hooks": [{ "type": "command", "command": "kiln hook stop", "timeout": 600, "statusMessage": "Running kiln" }] }
    ]
  }
}
```

### How the hook works

- When a session starts, kiln records the state of the working tree. When a reply ends, it checks only the files and lines changed since then. Files you were already editing before the session are compared against how they looked when it started.
- With `kiln hook track` (PreToolUse and PostToolUse) on, only files this session changed get checked. kiln records files that change right before and right after each tool call, plus any file an edit tool names, as belonging to this session. Files that other sessions or people change in the same working tree aren't checked, so several sessions can share one repo without blocking each other. A file another session changes at the same moment can still get attributed to this one. Files another session touched with an edit tool, or named in a command or script, are attributed to that session and left out. Files changed without naming the path (a wildcard, for example) can't be told apart. Giving each session its own `git worktree` avoids all of this.
- Without `track`, everything changed since the session started gets checked.
- kiln finds repos by the files that changed, not by the directory the session was opened in. A session opened in a parent directory (like `~/Desktop`), or one that edits several repos, gets each repo checked. The first time a session touches a repo, kiln uses the working tree at that moment as the baseline. For repos changed through shell commands or scripts, kiln goes by the paths in the command. A repo changed without its path appearing anywhere won't be found.
- If nothing changed, or nothing changed since the last pass, kiln skips the check.
- A pass prints nothing. Nothing goes into the model's context, so it costs no tokens.
- A failure blocks the reply from ending and hands the last 40 lines of output back to the model. Per project, kiln stops at the first command that fails.
- After blocking three times in a row in one session, kiln stops blocking and tells the user.
- State lives in `~/.local/state/kiln`. Anything older than 14 days is deleted when a session starts.

### Log

Every time the hook actually runs checks, it appends the result (pass, block, or give-up, which check failed, and how many issues) to `~/.local/state/kiln/log.jsonl`. `kiln stats` shows the time range, pass and block counts, how often a block was followed by a pass in the same session, and per-check block and issue counts.

## Development

    scripts/setup.sh     # install from source and enable commit checks for this repo
    scripts/check.sh     # format, lint, test

Pushing a `v*` tag runs the release workflow, which builds the binaries and uploads them to a GitHub Release.

## License

[MIT](LICENSE)
