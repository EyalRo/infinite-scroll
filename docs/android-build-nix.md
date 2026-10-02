# Building the Android app with Nix

How to build the debug APK from a machine that has Nix (including NixOS) and
put it on a phone for testing, without installing Android Studio or a global
JDK/SDK. Everything comes from `nix shell` / `nix build`.

> **Tested vs. untested.** The Nix expression and the tool attributes below
> were evaluated against current nixpkgs. The SDK download itself, the Gradle
> build and the install on a phone have **not** been run (the environment this
> was written in could not reach Google's download servers, and the app has
> not yet been compiled anywhere). If a step fails, see
> [Troubleshooting](#troubleshooting).

## One-time setup

Flakes and the new CLI must be enabled (NixOS: `nix.settings.experimental-features
= [ "nix-command" "flakes" ]`; otherwise `export NIX_CONFIG="experimental-features
= nix-command flakes"`).

## 1. Get the Android SDK

[`android/sdk.nix`](../android/sdk.nix) composes the SDK pieces the app needs
(platform 36, build-tools 36.0.0) and accepts the Android SDK licence, which
is why the build needs `--impure`. From the repository root:

```sh
cd android
export ANDROID_HOME="$(nix build --impure --no-link --print-out-paths \
  --expr 'import ./sdk.nix')/libexec/android-sdk"
echo "$ANDROID_HOME"
```

This downloads the SDK packages from Google on first use and then keeps them
in the Nix store. If you change `compileSdk` in `app/build.gradle.kts`, change
`platformVersions` / `buildToolsVersions` in `sdk.nix` to match.

## 2. Build

Gradle needs a JDK (17 or newer). `nix shell` provides one for the command and
nothing else on your system changes:

```sh
# Pure-Kotlin protocol tests first; these need no SDK:
nix shell nixpkgs#jdk17 -c ./gradlew :protocol:test

# The app itself:
nix shell nixpkgs#jdk17 -c ./gradlew :app:assembleDebug \
  -Pandroid.aapt2FromMavenOverride="$ANDROID_HOME/build-tools/36.0.0/aapt2"
```

The `aapt2FromMavenOverride` flag matters on NixOS: Gradle otherwise downloads
a prebuilt `aapt2` binary that cannot run there. Pointing it at the SDK's own
(Nix-patched) copy avoids that. On non-NixOS Nix installs it is harmless.

The APK is written to:

```text
app/build/outputs/apk/debug/app-debug.apk
```

The first build downloads Gradle and the Android/Kotlin/Compose dependencies,
so it needs network access and takes several minutes. `settings.gradle.kts`
only includes the `:app` module when `ANDROID_HOME` is set, which is why the
export in step 1 is required.

## 3. Install on the phone

### Option A — USB with `adb` (recommended)

On the phone: Settings → About phone → tap **Build number** seven times, then
Settings → System → Developer options → enable **USB debugging**. Plug in the
cable and accept the "Allow USB debugging?" prompt.

```sh
nix shell nixpkgs#android-tools -c adb devices          # phone should be "device", not "unauthorized"
nix shell nixpkgs#android-tools -c adb install -r app/build/outputs/apk/debug/app-debug.apk
```

`-r` reinstalls over an earlier build and keeps its data. Launch the app from
the phone's launcher, and accept the **Nearby devices** (Bluetooth) permission
on first run.

### Option B — wireless debugging (no cable)

Phone and workstation on the same Wi-Fi. On the phone: Developer options →
**Wireless debugging** → on → **Pair device with pairing code**.

```sh
nix shell nixpkgs#android-tools -c adb pair  <phone-ip>:<pairing-port>   # enter the 6-digit code
nix shell nixpkgs#android-tools -c adb connect <phone-ip>:<connect-port> # port shown on the Wireless debugging screen
nix shell nixpkgs#android-tools -c adb install -r app/build/outputs/apk/debug/app-debug.apk
```

### Option C — no `adb`: serve the file

```sh
cd app/build/outputs/apk/debug
nix run nixpkgs#python3 -- -m http.server 8000
```

On the phone's browser open `http://<workstation-ip>:8000/app-debug.apk`,
download it, and open it. Android will ask you to allow installs from that
browser ("Install unknown apps"). Stop the server when done.

## Iterating

Rebuild and reinstall in one go:

```sh
nix shell nixpkgs#jdk17 nixpkgs#android-tools -c bash -c '
  ./gradlew :app:assembleDebug \
    -Pandroid.aapt2FromMavenOverride="$ANDROID_HOME/build-tools/36.0.0/aapt2" &&
  adb install -r app/build/outputs/apk/debug/app-debug.apk'
```

Watch the app's Bluetooth activity on the phone with
`nix shell nixpkgs#android-tools -c adb logcat | grep -i bluetooth`.

## Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `error: ... android-sdk-license` or an unfree-package error | Run `nix build` with `--impure`; the licence is accepted in `sdk.nix`, which is only read in impure mode. |
| `SDK location not found` / Gradle only builds `:protocol` | `ANDROID_HOME` is not exported in the current shell (step 1). |
| `Could not start dir ... aapt2` / "stub-ld" / "Could not find the dynamic linker" | The `-Pandroid.aapt2FromMavenOverride=...` flag is missing or its path is wrong (check `ls "$ANDROID_HOME/build-tools"`). |
| `Failed to find Platform SDK with path: platforms;android-NN` or build-tools missing | `compileSdk` and the versions in `sdk.nix` disagree. Make them match, rebuild the SDK. |
| `adb devices` shows `unauthorized` | Unlock the phone and accept the USB-debugging prompt; or revoke USB debugging authorisations in Developer options and reconnect. |
| `INSTALL_FAILED_UPDATE_INCOMPATIBLE` | The phone has a build signed with a different key. `adb uninstall art.infinitescroll.control`, then install again. |
| The first compile reports Kotlin/Compose errors | Expected: the app module was written without an SDK and has never been compiled. Fix what the compiler reports (see `android/README.md`). |
