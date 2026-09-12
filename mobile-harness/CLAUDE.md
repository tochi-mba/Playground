# mobile-harness

Everything in this folder is one project: an Android sample app plus a harness that turns the user's phone into a self-hosted GitHub Actions
runner, so instrumented tests run on real hardware while Claude works from the cloud.

Gradle root is this folder, so run `./gradlew` from `mobile-harness/`, not the repo root.
The workflows live at the repo root under `.github/workflows/` and `cd` here; the Claude skill
lives at the repo root under `.claude/skills/device-test/`.

## Layout (paths relative to this folder)

- `app/`: minimal Jetpack Compose counter app. Pure logic in `Counter.kt`, UI in `MainActivity.kt`.
  JVM unit tests in `src/test`, instrumented tests in `src/androidTest`.
- `../.github/workflows/android.yml`: cloud build, unit tests, uploads the `apks` artifact.
- `../.github/workflows/device-test.yml`: manual dispatch; runs androidTest on the phone runner.
- `../.github/workflows/release.yml`: every push to `main` publishes a GitHub Release (`v0.1.<run>`)
  with signed release, debug and androidTest APKs attached. `workflow_dispatch` from another branch
  makes a `build-<run>` pre-release instead.
- `signing/debug.keystore`: public test-only key shared by all CI builds so updates install in place.
- `harness/`: phone-side scripts, Termux runner setup, and `harness/README.md` for the human steps.
- `../.claude/skills/device-test/SKILL.md`: how to drive a device run and read its results. Use it
  whenever a change needs real-device verification.

## Build

- JDK 17, Gradle wrapper 8.9, AGP 8.7.3, Kotlin 2.0.21, compileSdk 35, minSdk 26.
- `./gradlew testDebugUnitTest` for unit tests; `./gradlew assembleDebug assembleDebugAndroidTest`
  builds what the phone installs. The debug build uses applicationId `dev.tochi.playground.harness`.
- `versionCode`/`versionName` come from `VERSION_CODE`/`VERSION_NAME` env vars (CI sets them from the run number).
- The remote Claude container cannot reach dl.google.com, so Android builds are verified by CI on
  push, not locally. `harness/scripts/test_instrument_to_junit.py` (run from this folder) runs locally with plain Python.

## Conventions

- Keep tests on the JVM unless they need a device. Device runs interrupt the user's phone.
- Ask the user before dispatching `device-test.yml` (see the skill).
- Harness scripts must stay runner-agnostic: bash + python3 + adb only, no GitHub-specific calls.
