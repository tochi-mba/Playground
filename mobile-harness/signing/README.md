# Signing

`debug.keystore` is a throwaway key (password `android`, alias `androiddebugkey`) shared by
every debug and CI build. It exists so that each new APK from CI installs *over* the previous
one on your phone instead of failing with a signature mismatch. It is public by design.

For a real store release, add these repository secrets and the release workflow will use
them instead: `ANDROID_KEYSTORE_BASE64` (`base64 -w0 your.jks`), `ANDROID_KEYSTORE_PASSWORD`,
`ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`.
