---
name: device-test
description: Run the app's instrumented (androidTest) suite on the user's real phone through the self-hosted GitHub Actions runner, and read the results. Use after pushing Android changes that need real-device verification, or when the user asks to test on their phone.
---

# Device tests on the phone

The phone is a self-hosted GitHub Actions runner (label `android-phone`) living in Termux.
Claude never talks to the phone directly. Everything goes through GitHub:

1. `android.yml` (cloud) builds and uploads the `apks` artifact on every push.
2. `device-test.yml` (phone) is dispatched by hand with that build's run id.
3. Results come back as the `device-results` artifact.

## Procedure

1. **Push** the branch. Wait for the `Android` workflow run on that branch to complete
   successfully and note its run id (`actions_list` → `list_workflow_runs` for `android.yml`,
   filtered by branch). If it is red, fix the build first; there is nothing to test on the phone.
2. **Check the phone is online.** The GitHub MCP tools do not list runners, so use the dispatch
   itself as the probe: a dispatched device run that stays `queued` for more than 2 minutes means
   the runner is offline. Cancel it (`actions_run_trigger` → `cancel_workflow_run`) and tell the
   user: "Your phone runner is offline. Toggle Wireless debugging on and tap the Start Harness
   widget, then tell me when it shows Online." Do not dispatch again until they confirm.
3. **Ask before running.** The device is the user's daily phone and UI tests take over the
   screen for a minute or two. Ask "Ready for a device run? It will take over your screen for
   about two minutes." Skip the question only if the user said earlier in this session to run
   without asking.
4. **Dispatch** with `actions_run_trigger` → `run_workflow`, `workflow_id: device-test.yml`,
   `ref: <branch>`, `inputs: { build_run_id: "<id from step 1>" }`. Optionally
   `test_filter: "-e class dev.tochi.playground.CounterScreenTest"` to run a subset.
   Never dispatch while another device run is queued or in progress (concurrency group `phone`).
5. **Wait** for the run to complete (typically 1 to 3 minutes once picked up), then read the
   `device-results` artifact (`actions_list` → `list_workflow_run_artifacts`, then
   `actions_get` → `download_workflow_run_artifact`). Also read the job log if the run failed
   before producing results.
6. **Report** pass/fail counts from `summary.json`, and for each failure the test name plus the
   first lines of its stack from `junit.xml`. Consult `logcat.txt` for crashes and
   `screenshot.png` for the final screen state.

## Reading failures

| Symptom | Meaning | Action |
| --- | --- | --- |
| Job log: `Could not connect adb to this phone` | Wireless debugging is off (it resets on reboot) | Ask the user to toggle it on, then re-dispatch |
| Run stays `queued` | Runner offline | Cancel, ask the user to start the harness |
| `harness.instrumentation#run` error "could not start" | Test APK not installed or `APP_ID` in `device-test.yml` does not match the app | Check `APP_ID` and the `apks` artifact contents |
| `harness.instrumentation#run` error "did not complete" | App process crashed | Read the exception in `logcat.txt`; a test may also be marked "did not finish" |
| `<failure>` with an assertion stack | Normal test failure | Fix the code or the test |
| Job cancelled or timed out (30 min) | Phone slept, lost network, or a test hung | `device-cleanup.sh` still ran; check `runner.log` on the phone |

## What lives where

- `.github/workflows/android.yml`: cloud build. Change nothing here for device testing.
- `.github/workflows/device-test.yml`: phone job. `APP_ID` is the one knob when targeting a different app.
- `mobile-harness/harness/scripts/`: what the job runs on the phone. Plain bash + python, runner-agnostic.
- `mobile-harness/harness/termux/`: how the runner is installed and started. Setup steps in `mobile-harness/harness/README.md`.

Unit tests (`mobile-harness/app/src/test`) run in the cloud on every push and never need the phone. Only put
tests in `mobile-harness/app/src/androidTest` when they truly need a device.
