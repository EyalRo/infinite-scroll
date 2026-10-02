#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# rustup's rustc on a NixOS dev host needs zlib on LD_LIBRARY_PATH or it fails
# with "libz.so.1: cannot open shared object file". Resolved at run time (not
# hardcoded) since the exact store path is host- and gc-specific.
ZLIB_LIB="$(nix build nixpkgs#zlib --no-link --print-out-paths)/lib"

# Everything needed to build comes from nix, so this works from a clean shell:
# rustup (cargo/rustc proxies), a native gcc (host linker for build scripts) and
# the aarch64 cross compiler.
NIX_TOOLS=(nixpkgs#rustup nixpkgs#gcc nixpkgs#pkgsCross.aarch64-multiplatform.stdenv.cc)

echo "Preparing Rust toolchain..."
nix shell "${NIX_TOOLS[@]}" -c env LD_LIBRARY_PATH="$ZLIB_LIB" bash -c '
  rustup show active-toolchain >/dev/null 2>&1 || rustup default stable
  rustup target add aarch64-unknown-linux-gnu'

echo "Cross-compiling..."
nix shell "${NIX_TOOLS[@]}" -c env \
  LD_LIBRARY_PATH="$ZLIB_LIB" \
  CC_aarch64_unknown_linux_gnu=aarch64-unknown-linux-gnu-gcc \
  cargo build --release --target aarch64-unknown-linux-gnu -p uploader -p watcher -p printer

# btcontrol (Bluetooth) is built separately and is optional for the deploy: it
# needs a vendored libdbus cross-build, and a failure there must never block
# shipping the web-facing services. The deploy still ends non-zero if it fails.
BTCONTROL_OK=1
nix shell "${NIX_TOOLS[@]}" -c env \
  LD_LIBRARY_PATH="$ZLIB_LIB" \
  CC_aarch64_unknown_linux_gnu=aarch64-unknown-linux-gnu-gcc \
  cargo build --release --target aarch64-unknown-linux-gnu -p bluetooth || {
    BTCONTROL_OK=0
    echo "WARNING: btcontrol failed to build; deploying uploader/watcher/printer only." >&2
  }
SERVICES="uploader watcher printer"
[ "$BTCONTROL_OK" = 1 ] && SERVICES="$SERVICES btcontrol"

HOST=infinite-scroll.local
BIN_DIR=target/aarch64-unknown-linux-gnu/release
PATCHED_DIR=$(mktemp -d)
trap 'rm -rf "$PATCHED_DIR"' EXIT

# nixpkgs' cross-compiled aarch64-gnu binaries carry a PT_INTERP pointing at
# their own Nix store ld-linux path, which doesn't exist on the Pi's Debian
# filesystem (execve fails with ENOENT/"required file not found"). Re-point
# the interpreter at the Pi's real dynamic linker before shipping. Symbol
# versioning is not an issue in practice here (verified working against the
# Pi's older glibc 2.41 despite building against nixpkgs' glibc 2.42).
echo "Patching ELF interpreters for the target's glibc..."
for name in $SERVICES; do
  cp "$BIN_DIR/$name" "$PATCHED_DIR/$name"
  nix shell nixpkgs#patchelf -c patchelf --set-interpreter /lib/ld-linux-aarch64.so.1 "$PATCHED_DIR/$name"
done

echo "Copying binaries..."
ssh "$HOST" "mkdir -p /home/stags/infinite-scroll/bin"
for name in $SERVICES; do
  scp "$PATCHED_DIR/$name" "$HOST:/home/stags/infinite-scroll/bin/$name.new"
  ssh "$HOST" "mv /home/stags/infinite-scroll/bin/$name.new /home/stags/infinite-scroll/bin/$name && chmod +x /home/stags/infinite-scroll/bin/$name"
done

# Space-joined lists: a newline-separated $(...) inside the remote command would
# split it into several commands.
UNIT_FILES=""
UNIT_NAMES=""
for n in $SERVICES; do
  UNIT_FILES="$UNIT_FILES deploy/$n.service"
  UNIT_NAMES="$UNIT_NAMES $n.service"
done

echo "Copying systemd units..."
scp $UNIT_FILES "$HOST:/tmp/"
ssh "$HOST" "cd /tmp && sudo mv $UNIT_NAMES /etc/systemd/system/ && sudo systemctl daemon-reload"

if [ "$BTCONTROL_OK" = 1 ]; then
  echo "Preparing Bluetooth on the Pi..."
  # btcontrol switches Wi-Fi through NetworkManager; see the rule for why polkit needs this.
  scp deploy/50-infinite-scroll-wifi.rules "$HOST:/tmp/"
  ssh "$HOST" "sudo install -m 644 -o root -g root /tmp/50-infinite-scroll-wifi.rules /etc/polkit-1/rules.d/50-infinite-scroll-wifi.rules && rm -f /tmp/50-infinite-scroll-wifi.rules"
  # btcontrol's EnvironmentFile: the same tokens as printer.env/uploader.env.
  # Created on the Pi from them if missing; token values never leave the Pi.
  ssh "$HOST" 'if ! sudo test -f /etc/infinite-scroll/bluetooth.env; then
    sudo sh -c "umask 077; { grep ^PRINTER_TOKEN= /etc/infinite-scroll/printer.env; grep ^UPLOADER_TOKEN= /etc/infinite-scroll/uploader.env; } > /etc/infinite-scroll/bluetooth.env"
  fi
  # The adapter can come up soft-blocked by rfkill; powering it on unblocks it.
  sudo bluetoothctl power on >/dev/null || echo "WARNING: could not power on the Bluetooth adapter" >&2'
fi

echo "Restarting services..."
ssh "$HOST" "sudo systemctl enable --now $SERVICES && sudo systemctl reset-failed $SERVICES; sudo systemctl restart $SERVICES"

echo "Health checks..."
sleep 3
ssh "$HOST" "curl -sf http://127.0.0.1:8081/health && echo ' uploader ok'"
ssh "$HOST" "curl -sf http://127.0.0.1:8082/health && echo ' printer ok'"
if [ "$BTCONTROL_OK" = 1 ]; then
  ssh "$HOST" "systemctl is-active --quiet btcontrol && echo ' btcontrol ok'"
  echo "Deploy complete."
else
  echo "Deploy complete for uploader/watcher/printer; btcontrol was NOT deployed (build failed)." >&2
  exit 1
fi
