#!/usr/bin/env bash
# Build the Next.js frontend as static export output (frontend/out).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FORCE=0
if [[ "${1:-}" == "--force" ]]; then
    FORCE=1
fi

cd "$ROOT/frontend"

if [[ "$FORCE" == "1" ]]; then
    echo "==> Removing previous frontend build output..."
    rm -rf .next out
fi

if [[ ! -d node_modules || ! -f node_modules/.package-lock.json || \
      package-lock.json -nt node_modules/.package-lock.json ]]; then
    echo "==> Installing frontend dependencies..."
    if ! npm ci --no-audit --no-fund; then
        echo "ERROR: npm ci failed. Set HTTP_PROXY/HTTPS_PROXY if the network requires a proxy." >&2
        exit 1
    fi
fi

echo "==> Building frontend..."
NEXT_TELEMETRY_DISABLED=1 npm run build

if [[ ! -f "$ROOT/frontend/out/index.html" ]]; then
    echo "ERROR: frontend build did not produce out/index.html" >&2
    exit 1
fi

echo "==> Frontend ready: $ROOT/frontend/out"
