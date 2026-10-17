#!/usr/bin/env bash
# Cross-build static musl binaries inside a Rust Alpine container.
#
# The container runs on the host architecture (arm64 on this machine), so no
# qemu emulation is used. Both aarch64 and x86_64 targets are built with the
# pure-Rust dependency set; rust-lld is used as the cross linker.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PODMAN="${PODMAN:-podman}"
# shellcheck source=scripts/build-env.sh
source "$ROOT/scripts/build-env.sh"

BIN_DIR="$ROOT/bin"
mkdir -p "$BIN_DIR"

if [[ ! -f "$ROOT/frontend/out/index.html" ]]; then
    echo "ERROR: frontend/out is missing. Run 'make frontend' first." >&2
    exit 1
fi

if [[ -z "${PODMAN_OPTS:-}" ]]; then
    PODMAN_OPTS=""
    [[ -n "${PODMAN_ROOT:-}" ]] && PODMAN_OPTS+=" --root ${PODMAN_ROOT}"
    [[ -n "${PODMAN_RUNROOT:-}" ]] && PODMAN_OPTS+=" --runroot ${PODMAN_RUNROOT}"
fi

read -r -a PODMAN_ARGS <<< "${PODMAN_OPTS:-}"

echo "==> Cross-building static musl binaries in rust:1.94-alpine..."
rm -f "$BIN_DIR/bmp-status-linux-amd64-musl" "$BIN_DIR/bmp-status-linux-arm64-musl"

# The script runs inside the container. Keep variables expanded by the
# container shell, not by the host shell.
INNER_SCRIPT="$(cat <<'INNER'
set -eu

echo "--> Configuring the CERNET Alpine mirror: ${ALPINE_MIRROR}"
sed -i "s|dl-cdn.alpinelinux.org|${ALPINE_MIRROR}|g" /etc/apk/repositories

echo "--> Installing native build tools..."
apk add --no-cache build-base ca-certificates bash zig

echo "--> Installing Rust targets..."
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl

HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
LLD="$(rustc --print sysroot)/lib/rustlib/${HOST_TRIPLE}/bin/rust-lld"
if [ ! -x "$LLD" ]; then
    LLD="$(command -v rust-lld || true)"
fi
if [ -z "$LLD" ] || [ ! -x "$LLD" ]; then
    echo "ERROR: rust-lld was not found in the Rust toolchain." >&2
    exit 1
fi
echo "--> Linker: ${LLD}"

export BMP_SKIP_FRONTEND=1
export CARGO_TARGET_DIR=/tmp/target
export CARGO_NET_RETRY=5

# Apply rust-lld only to the target that differs from the container host.
# Applying it to the host target would also affect host proc-macros/build
# scripts, which need the native musl C runtime (libc/libgcc_s).
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
    aarch64|arm64)
        cat > /usr/local/bin/x86_64-linux-musl-cc <<'ZIGCC'
#!/bin/bash
set -euo pipefail
# cc-rs passes the Rust target triple as --target=x86_64-unknown-linux-musl.
# Zig's clang driver uses the shorter x86_64-linux-musl spelling and rejects
# the "unknown" OS part, so remove cc-rs' target flag and set Zig's own.
args=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --target=*|-target=*) shift ;;
        --target|-target)
            shift
            [[ $# -gt 0 ]] && shift
            ;;
        *) args+=("$1"); shift ;;
    esac
done
exec zig cc -target x86_64-linux-musl "${args[@]}"
ZIGCC
        chmod 0755 /usr/local/bin/x86_64-linux-musl-cc
        export CC_x86_64_unknown_linux_musl=/usr/local/bin/x86_64-linux-musl-cc
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$LLD"
        ;;
    x86_64|amd64)
        cat > /usr/local/bin/aarch64-linux-musl-cc <<'ZIGCC'
#!/bin/bash
set -euo pipefail
args=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --target=*|-target=*) shift ;;
        --target|-target)
            shift
            [[ $# -gt 0 ]] && shift
            ;;
        *) args+=("$1"); shift ;;
    esac
done
exec zig cc -target aarch64-linux-musl "${args[@]}"
ZIGCC
        chmod 0755 /usr/local/bin/aarch64-linux-musl-cc
        export CC_aarch64_unknown_linux_musl=/usr/local/bin/aarch64-linux-musl-cc
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="$LLD"
        ;;
    *)
        echo "ERROR: unsupported container host architecture: $HOST_ARCH" >&2
        exit 1
        ;;
esac

echo "--> Building x86_64-unknown-linux-musl..."
cargo build --release --locked --target x86_64-unknown-linux-musl \
    --manifest-path /work/backend/Cargo.toml

echo "--> Building aarch64-unknown-linux-musl..."
cargo build --release --locked --target aarch64-unknown-linux-musl \
    --manifest-path /work/backend/Cargo.toml

cp /tmp/target/x86_64-unknown-linux-musl/release/bmp-status \
    /out/bmp-status-linux-amd64-musl
cp /tmp/target/aarch64-unknown-linux-musl/release/bmp-status \
    /out/bmp-status-linux-arm64-musl
chmod 0755 /out/bmp-status-linux-amd64-musl /out/bmp-status-linux-arm64-musl

echo "--> Cross-build complete."
INNER
)"

"${PODMAN}" "${PODMAN_ARGS[@]}" run --rm --network=host \
    -e HTTP_PROXY="${HTTP_PROXY:-}" \
    -e HTTPS_PROXY="${HTTPS_PROXY:-}" \
    -e http_proxy="${http_proxy:-}" \
    -e https_proxy="${https_proxy:-}" \
    -e NO_PROXY="${NO_PROXY:-}" \
    -e no_proxy="${no_proxy:-}" \
    -e ALPINE_MIRROR="${ALPINE_MIRROR}" \
    -v "$ROOT/backend:/work/backend:ro,Z" \
    -v "$ROOT/frontend/out:/work/frontend/out:ro,Z" \
    -v "$BIN_DIR:/out:Z" \
    -w /work \
    rust:1.94-alpine /bin/sh -c "$INNER_SCRIPT"

for arch in amd64 arm64; do
    binary="$BIN_DIR/bmp-status-linux-${arch}-musl"
    if [[ ! -f "$binary" ]]; then
        echo "ERROR: expected artifact is missing: $binary" >&2
        exit 1
    fi
    echo "--> $binary"
    if command -v file >/dev/null 2>&1; then
        file "$binary"
        if file "$binary" | grep -qi 'dynamically linked'; then
            echo "WARNING: $binary is dynamically linked; expected a static musl binary." >&2
        fi
    else
        echo "    (file command not installed; skipping architecture check)" >&2
    fi
done

(
    cd "$BIN_DIR"
    sha256sum bmp-status-linux-* > SHA256SUMS
)

echo "==> Cross binaries:"
ls -lh "$BIN_DIR"/bmp-status-linux-*-musl
