# Playground

Scratch repository for experiments. Each experiment lives in its own top-level folder with its
own CLAUDE.md; read that file before working inside it.

- `mobile-harness/`: Android sample app plus the phone-as-CI-runner harness. Gradle root is that
  folder. Its workflows are in `.github/workflows/` (android.yml, device-test.yml, release.yml)
  and its Claude skill in `.claude/skills/device-test/`, because GitHub and Claude Code only read
  those from the repo root.
- `lumen/` used to hold the lumap projection-mapping app; it now lives in its own repository,
  https://github.com/tochi-mba/lumap (a REX Technologies product).

Workflows are path-filtered to their folder, so changes in one experiment do not trigger the
other's builds. Release tags and artifact names are prefixed with the project slug
(`mobile-harness-v0.1.4`) so each experiment can publish its own releases independently.
