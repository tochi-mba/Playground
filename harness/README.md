# Phone harness

Turns an Android phone into a self-hosted GitHub Actions runner so Claude Code (running in the
cloud) can run instrumented tests on real hardware without ever needing to reach the phone
directly. The phone only needs outbound internet, so it works whether the phone is the hotspot
or a client on someone else's Wi-Fi.

```
push branch ──> android.yml (cloud) builds app-debug.apk + androidTest.apk
                       │
   Claude dispatches device-test.yml with build_run_id
                       │
   runner in Termux picks the job up ──> adb over loopback ──> am instrument
                       │
   device-results artifact (junit.xml, logcat.txt, screenshot.png) ──> Claude reads it
```

## One-time setup (about 15 minutes)

1. Install from F-Droid (the Play Store build is abandoned): **Termux**, **Termux:API**,
   **Termux:Boot**, **Termux:Widget**. Open Termux:Boot once so Android lets it run at boot.
2. Phone settings: enable Developer options, then **Wireless debugging**. Also set Termux to
   *Unrestricted* under Settings > Apps > Termux > Battery, or Android will kill the runner.
3. In Termux:
   ```sh
   pkg install -y git
   git clone https://github.com/tochi-mba/playground ~/playground
   ```
4. Get a runner registration token at
   https://github.com/tochi-mba/playground/settings/actions/runners/new (linux, ARM64).
   It expires in an hour. Then:
   ```sh
   RUNNER_TOKEN=<paste token> ~/playground/harness/termux/install.sh
   ```
5. Pair adb with the phone itself. Wireless debugging > *Pair device with pairing code* shows
   an IP:port and a code:
   ```sh
   ~/playground/harness/termux/ubuntu.sh adb pair 127.0.0.1:<pairing port>
   ```
6. Start it: tap the **Start Harness** widget, or run
   `~/playground/harness/termux/start-runner.sh`. The runner should show **Online** at
   https://github.com/tochi-mba/playground/settings/actions/runners within a minute.

## Every reboot

Android turns Wireless debugging off on reboot and a non-root app cannot turn it back on.
So after a reboot: toggle **Wireless debugging** on. Termux:Boot restarts the runner by itself;
if it did not, tap the widget. The port changes every time, and `scripts/adb-connect.sh` finds it
again (saved port, then mDNS, then a loopback port scan). If everything fails it prints exactly
what to do.

## Day to day

- Claude asks before dispatching a run because the tests take over the screen for a minute or
  two. A notification appears when a run starts and when it ends.
- Unit tests never touch the phone; they run in the cloud build.
- Updating the harness is just merging a commit. `start-runner.sh` pulls the repo on every
  start and the workflow checks out the scripts fresh on every job.
- Logs: `~/.harness/runner.log` in Termux.

## Installing the app itself on your phone

Every push to `main` publishes a GitHub Release at
https://github.com/tochi-mba/playground/releases with three APKs: the release build, the
debug build the harness tests, and the androidTest APK. Download the release APK on the phone
and open it; each new version installs over the previous one because all CI builds share the
key in `signing/debug.keystore`.

## Pointing it at another app

Change `APP_ID` in `.github/workflows/device-test.yml` to the debug `applicationId` of the app
and make sure its build workflow uploads an artifact named `apks` containing the app APK and
the androidTest APK. Everything else is generic.

## Known limits

- **Foreground steal.** UI tests need the screen on and in front. If you are chatting with Claude
  on this phone, the run interrupts you. An old spare phone on a charger avoids this.
- **Per-reboot tap.** See above. Root would remove it; nothing else does.
- **.NET under proot.** The Actions runner is .NET. It is reported to work under proot on arm64
  but is not something GitHub supports. `PROOT_NO_SECCOMP=1` and invariant globalization are set
  to dodge the two most common failures. If `run.sh` will not start on your device, the fallback
  is a small Python poller that reuses `harness/scripts/` unchanged; open an issue with the log.
- **Data.** Each run downloads both APKs over the phone's uplink (a few MB for this sample).
