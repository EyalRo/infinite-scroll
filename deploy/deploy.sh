#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# rustup's rustc on a NixOS dev host needs zlib on LD_LIBRARY_PATH or it fails
# with "libz.so.1: cannot open shared object file". Resolved at run time (not
# hardcoded) since the exact store path is host- and gc-specific.
ZLIB_LIB="$(nix build nixpkgs#zlib --no-link --print-out-paths)/lib"

echo "Cross-compiling..."
nix shell nixpkgs#pkgsCross.aarch64-multiplatform.stdenv.cc -c env \
  LD_LIBRARY_PATH="$ZLIB_LIB" \
  CC_aarch64_unknown_linux_gnu=aarch64-unknown-linux-gnu-gcc \
  cargo build --release --target aarch64-unknown-linux-gnu -p uploader -p watcher -p printer

# btcontrol (Bluetooth) is built separately and is optional for the deploy: it
# needs a vendored libdbus cross-build, and a failure there must never block
# shipping the web-facing services. The deploy still ends non-zero if it fails.
BTCONTROL_OK=1
nix shell nixpkgs#pkgsCross.aarch64-multiplatform.stdenv.cc -c env \
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

echo "Copying systemd units..."
scp $(for n in $SERVICES; do echo deploy/$n.service; done) "$HOST:/tmp/"
ssh "$HOST" "cd /tmp && sudo mv $(for n in $SERVICES; do echo $n.service; done) /etc/systemd/system/ && sudo systemctl daemon-reload"

echo "Restarting services..."
ssh "$HOST" "sudo systemctl enable --now $SERVICES && sudo systemctl restart $SERVICES"

echo "Health checks..."
ssh "$HOST" "curl -sf http://127.0.0.1:8081/health && echo ' uploader ok'"
ssh "$HOST" "curl -sf http://127.0.0.1:8082/health && echo ' printer ok'"
if [ "$BTCONTROL_OK" = 1 ]; then
  ssh "$HOST" "systemctl is-active --quiet btcontrol && echo ' btcontrol ok'"
  echo "Deploy complete."
else
  echo "Deploy complete for uploader/watcher/printer; btcontrol was NOT deployed (build failed)." >&2
  exit 1
fi
