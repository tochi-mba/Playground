# Playground

Scratch repository for experiments. Each experiment lives in its own top-level folder with its
own CLAUDE.md; read that file before working inside it.

- `mobile-harness/`: Android sample app plus the phone-as-CI-runner harness. Gradle root is that
  folder. Its workflows are in `.github/workflows/` (android.yml, device-test.yml, release.yml)
  and its Claude skill in `.claude/skills/device-test/`, because GitHub and Claude Code only read
  those from the repo root.

Workflows are path-filtered to their folder, so changes elsewhere do not trigger Android builds.
