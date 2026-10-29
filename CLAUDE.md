# Playground

Scratch repository for experiments. Each experiment lives in its own top-level folder with its
own CLAUDE.md; read that file before working inside it.

- `mobile-harness/`: Android sample app plus the phone-as-CI-runner harness. Gradle root is that
  folder. Its workflows are in `.github/workflows/` (android.yml, device-test.yml, release.yml)
  and its Claude skill in `.claude/skills/device-test/`, because GitHub and Claude Code only read
  those from the repo root.
- `lumen/`: projection-mapping app in Rust (egui editor, wgpu renderer, headless CLI). Cargo
  root is that folder. Its workflows are `lumen-ci.yml` and `lumen-release.yml` and its Claude
  skill is `.claude/skills/lumen-render/`.

Workflows are path-filtered to their folder, so changes in one experiment do not trigger the
other's builds. Release tags and artifact names are prefixed with the project slug
(`mobile-harness-v0.1.4`, `lumen-v0.1.0`) so each experiment can publish its own releases
independently.
