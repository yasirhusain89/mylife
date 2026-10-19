#!/usr/bin/env bash
# Builds the fotoflow engine sidecar and places it where Tauri expects it.
#
# Toolchain selection. CLT ships a swiftc whose version matches the macOS SDK it
# ships with; older CLT builds (swiftc 6.2.4 vs an SDK built by 6.2.3) reject the
# SDK outright. A local swift.org toolchain at ~/toolchains/swift-* was used to
# work around that; it is no longer needed now that CLT matches the SDK.
#
# Order: $SWIFTC > system swiftc (xcrun) > ~/toolchains fallback.
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
elif SWIFTC_SYSTEM="$(xcrun --find swiftc 2>/dev/null)" && [[ -x "$SWIFTC_SYSTEM" ]]; then
  SWIFTC_BIN="$SWIFTC_SYSTEM"
elif [[ -x "$HOME/toolchains/swift-6.2.3/usr/bin/swiftc" ]]; then
  echo "warning: no system swiftc found; falling back to ~/toolchains/swift-6.2.3" >&2
  SWIFTC_BIN="$HOME/toolchains/swift-6.2.3/usr/bin/swiftc"
else
  echo "error: no Swift compiler found (set SWIFTC=/path/to/swiftc)" >&2
  exit 1
fi

echo "swiftc: $SWIFTC_BIN ($("$SWIFTC_BIN" --version 2>&1 | head -1))"

SDK="$(xcrun --show-sdk-path)"
mkdir -p src-tauri/binaries

# Placeholder protocol version marker; bump when engine protocol changes.
# -g keeps line info in crash reports (stripped only for release builds).
"$SWIFTC_BIN" \
  -O \
  -g \
  -sdk "$SDK" \
  -target "${ARCH}-apple-macos15.0" \
  sidecar/engine/Sources/engine/main.swift \
  -o "src-tauri/binaries/engine-${TARGET}"

echo "engine -> src-tauri/binaries/engine-${TARGET}"
