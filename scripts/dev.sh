#!/usr/bin/env bash
# Development mode: Rust backend on :8787 + Next.js HMR dev server on :3000.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT/backend"

if [[ ! -d "$ROOT/frontend/node_modules" ]]; then
  echo "==> installing frontend dependencies"
  (cd "$ROOT/frontend" && npm install --no-audit --no-fund)
fi

# build.rs also builds frontend/out; --frontend-dir lets the backend serve the
# on-disk build while we iterate on the Next.js dev server.
cargo run -- \
  --frontend-dir "$ROOT/frontend/out" \
  --data-dir "$ROOT/data" \
  "$@" &
BACKEND_PID=$!

cleanup() {
  kill "$BACKEND_PID" 2>/dev/null || true
  if [[ -n "${FRONTEND_PID:-}" ]]; then
    kill "$FRONTEND_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

cd "$ROOT/frontend"
npm run dev -- --port 3000 &
FRONTEND_PID=$!

echo "==> dashboard: http://localhost:3000  backend: http://localhost:8787  admin: http://localhost:3000/admin/"
wait -n "$BACKEND_PID" "$FRONTEND_PID"
