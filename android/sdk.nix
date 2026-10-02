# Android SDK for building the app with Nix; see docs/android-build-nix.md.
# Accepting the licence here is what allows the unfree SDK packages to build.
# Platform and build-tools versions must match compileSdk in app/build.gradle.kts.
let
  pkgs = import (builtins.getFlake "nixpkgs") {
    config = { allowUnfree = true; android_sdk.accept_license = true; };
  };
in
(pkgs.androidenv.composeAndroidPackages {
  platformVersions = [ "36" ];
  buildToolsVersions = [ "36.0.0" ];
  includeEmulator = false;
  includeSystemImages = false;
}).androidsdk
