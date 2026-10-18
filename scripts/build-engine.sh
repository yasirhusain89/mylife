#!/usr/bin/env bash
# Builds the fotoflow engine sidecar and places it where Tauri expects it.
#
# This machine: CLT ships swiftc 6.2.4 but the MacOSX26.2 SDK was built with
# Swift 6.2.3 (swiftlang-6.2.3.3.2) — the 6.2.4 compiler rejects it. The
# matched 6.2.3 toolchain from swift.org is installed at ~/toolchains/swift-6.2.3;
# SWIFTC env overrides, plain `swiftc` used only as a last resort (may mismatch).
set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="$(uname -m)"
case "$ARCH" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

if [[ -n "${SWIFTC:-}" ]]; then
  SWIFTC_BIN="$SWIFTC"
elif [[ -x "$HOME/toolchains/swift-6.2.3/usr/bin/swiftc" ]]; then
  SWIFTC_BIN="$HOME/toolchains/swift-6.2.3/usr/bin/swiftc"
else
  echo "warning: matched Swift 6.2.3 toolchain not found at ~/toolchains/swift-6.2.3;" \
       "falling back to PATH swiftc (compiler/SDK skew may fail the build)" >&2
  SWIFTC_BIN="swiftc"
fi

SDK="$(xcrun --show-sdk-path)"
mkdir -p src-tauri/binaries

# Placeholder protocol version marker; bump when engine protocol changes.
"$SWIFTC_BIN" \
  -O \
  -sdk "$SDK" \
  -target "${ARCH}-apple-macos15.0" \
  sidecar/engine/Sources/engine/main.swift \
  -o "src-tauri/binaries/engine-${TARGET}"

echo "engine -> src-tauri/binaries/engine-${TARGET}"
