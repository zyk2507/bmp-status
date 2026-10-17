#!/usr/bin/env bash
# Build the backend with the local Rust toolchain and copy the native binary to bin/.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FORCE=0
if [[ "${1:-}" == "--force" ]]; then
    FORCE=1
fi

cd "$ROOT/backend"

if [[ "$FORCE" == "1" ]]; then
    echo "==> Cleaning the bmp-status package..."
    cargo clean -p bmp-status || true
fi

echo "==> Building backend with the native Rust toolchain..."
cargo build --release --locked

case "$(uname -m)" in
    x86_64|amd64)
        ARCH="amd64"
        ;;
    aarch64|arm64)
        ARCH="arm64"
        ;;
    *)
        ARCH="$(uname -m)"
        ;;
esac

BIN_DIR="$ROOT/bin"
mkdir -p "$BIN_DIR"
OUT="$BIN_DIR/bmp-status-linux-${ARCH}-native"
install -m 0755 "$ROOT/backend/target/release/bmp-status" "$OUT"

(
    cd "$BIN_DIR"
    sha256sum bmp-status-linux-* > SHA256SUMS
)

echo "==> Native binary: $OUT"
ls -lh "$OUT"
