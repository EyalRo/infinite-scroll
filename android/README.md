# Infinite Scroll — Android controller

Native BLE/GATT client for the deployed, offline installation. It is a
control panel only: printing, scheduling, the library and image processing
all stay on the Pi; the app sends requests defined in
`../docs/ble-protocol.md` and shows the answers. It distinguishes
**accepted** (the Pi now owns the work — safe to disconnect) from
**completed**.

Target: Pixel on Android 17; `minSdk 33`.

## Layout

- `protocol/` — pure Kotlin/JVM: framing, wire types, models, the RPC
  client, the resumable uploader. No Android dependency, so it is built and
  unit-tested anywhere: `gradle :protocol:test`.
- `app/` — Android module: `GattTransport` (one GATT operation at a time),
  scanner, view-model, Compose screens (Status, Library, Printing,
  Schedule, Printer). Included in the build only when an Android SDK is
  configured (`ANDROID_HOME` or `local.properties`).

## Build

On Nix or NixOS, see [`docs/android-build-nix.md`](../docs/android-build-nix.md) for a no-install route (SDK, JDK and `adb` from `nix shell`).


```sh
./gradlew :protocol:test          # anywhere
./gradlew :app:assembleDebug      # with the Android SDK installed
```

## Status

The `protocol` module is tested (framing and upload vectors shared with the
Rust service, RPC and upload-resume against a scripted fake Pi). The `app`
module has **not been compiled or run** — it was written without an Android
SDK or a device — so expect a first-build fix-up pass, and verify the BLE
flow against the real Pi. `compileSdk`/`targetSdk` are 36; raise to 37 when
that platform is installed.
