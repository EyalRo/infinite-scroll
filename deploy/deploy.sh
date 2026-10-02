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
  cargo build --release --target aarch64-unknown-linux-gnu --workspace

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
for name in uploader watcher printer btcontrol; do
  cp "$BIN_DIR/$name" "$PATCHED_DIR/$name"
  nix shell nixpkgs#patchelf -c patchelf --set-interpreter /lib/ld-linux-aarch64.so.1 "$PATCHED_DIR/$name"
done

echo "Copying binaries..."
ssh "$HOST" "mkdir -p /home/stags/infinite-scroll/bin"
for name in uploader watcher printer btcontrol; do
  scp "$PATCHED_DIR/$name" "$HOST:/home/stags/infinite-scroll/bin/$name.new"
  ssh "$HOST" "mv /home/stags/infinite-scroll/bin/$name.new /home/stags/infinite-scroll/bin/$name && chmod +x /home/stags/infinite-scroll/bin/$name"
done

echo "Copying systemd units..."
scp deploy/uploader.service deploy/watcher.service deploy/printer.service deploy/btcontrol.service "$HOST:/tmp/"
ssh "$HOST" "sudo mv /tmp/uploader.service /tmp/watcher.service /tmp/printer.service /tmp/btcontrol.service /etc/systemd/system/ && sudo systemctl daemon-reload"

echo "Restarting services..."
ssh "$HOST" "sudo systemctl enable --now uploader watcher printer btcontrol && sudo systemctl restart uploader watcher printer btcontrol"

echo "Health checks..."
ssh "$HOST" "curl -sf http://127.0.0.1:8081/health && echo ' uploader ok'"
ssh "$HOST" "curl -sf http://127.0.0.1:8082/health && echo ' printer ok'"
ssh "$HOST" "systemctl is-active --quiet btcontrol && echo ' btcontrol ok'"
echo "Deploy complete."
