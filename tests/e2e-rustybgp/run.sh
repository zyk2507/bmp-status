#!/usr/bin/env bash
# End-to-end test: two RustyBGP routers exchange routes replayed from a cached
# RIPE RIS MRT file. Router r1 sends BMP (pre-policy) to this project.
#
# SAFETY: this test must never write to the host routing table.
#   * RustyBGP's kernel/FIB integration and NHT are only enabled by the
#     `zebra` config section (or the EnableZebra gRPC call). The configs below
#     deliberately contain NO zebra section.
#   * The script refuses to run as root.
#   * It snapshots the complete IPv4/IPv6 route tables before/after and fails
#     on any difference.
#   * It verifies the RustyBGP processes have no CAP_NET_ADMIN and that their
#     logs never say "kernel route integration enabled".
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUN_DIR="${RUN_DIR:-$ROOT/.cache/e2e-rustybgp}"
RUSTYBGP_DIR="${RUSTYBGP_DIR:-$ROOT/.cache/rustybgp}"
RUSTYBGP_REPO="${RUSTYBGP_REPO:-https://github.com/osrg/rustybgp.git}"
RUSTYBGP_BIN="${RUSTYBGP_BIN:-$RUSTYBGP_DIR/target/debug/rustybgpd}"

BMP_BIND="${BMP_BIND:-127.0.0.1:11019}"
HTTP_BIND="${HTTP_BIND:-127.0.0.1:8787}"
R1_BGP_PORT="${R1_BGP_PORT:-1179}"
R2_BGP_PORT="${R2_BGP_PORT:-2179}"
R1_API_PORT="${R1_API_PORT:-50051}"
R2_API_PORT="${R2_API_PORT:-50052}"

ADMIN_PASSWORD="${ADMIN_PASSWORD:-e2e-admin-pass-123}"
RIPE_URL="${RIPE_URL:-https://data.ris.ripe.net/rrc26/2025.01/updates.20250101.0000.gz}"
RIPE_CACHE="${RIPE_CACHE:-$ROOT/data/ripe-cache}"
LIMIT="${LIMIT:-2000}"
LIMIT_V6="${LIMIT_V6:-300}"
MIN_ROUTES="${MIN_ROUTES:-100}"
KEEP_RUNNING="${KEEP_RUNNING:-0}"

BMP_HOST="${BMP_BIND%:*}"
BMP_PORT="${BMP_BIND##*:}"
HTTP_HOST="${HTTP_BIND%:*}"
HTTP_PORT="${HTTP_BIND##*:}"
[[ "$BMP_HOST" == "0.0.0.0" || "$BMP_HOST" == "::" ]] && BMP_CLIENT_HOST="127.0.0.1" || BMP_CLIENT_HOST="$BMP_HOST"
[[ "$HTTP_HOST" == "0.0.0.0" || "$HTTP_HOST" == "::" ]] && HTTP_PROBE_HOST="127.0.0.1" || HTTP_PROBE_HOST="$HTTP_HOST"
HTTP_PROBE="http://$HTTP_PROBE_HOST:$HTTP_PORT"

COLLECTOR_PID=""
R1_PID=""
R2_PID=""

log() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
pass() { printf '  \033[1;32mPASS\033[0m  %s\n' "$*"; }
fail() {
  printf '\n\033[1;31mFAIL: %s\033[0m\n' "$*" >&2
  for name in collector r1 r2; do
    if [[ -f "$RUN_DIR/$name.log" ]]; then
      printf '\n--- %s.log (tail) ---\n' "$name" >&2
      tail -25 "$RUN_DIR/$name.log" >&2 || true
    fi
  done
  exit 1
}

cleanup() {
  local code=$?
  for pid in "$R2_PID" "$R1_PID" "$COLLECTOR_PID"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  sleep 0.5
  for pid in "$R2_PID" "$R1_PID" "$COLLECTOR_PID"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill -9 "$pid" 2>/dev/null || true
    fi
  done
  exit "$code"
}
trap cleanup EXIT INT TERM

port_in_use() {
  ss -ltnH 2>/dev/null | awk '{print $4}' | grep -qE "[:.]$1$"
}

# ---------------------------------------------------------------------------
# Safety guards: this is a simulation, not a real router.
# ---------------------------------------------------------------------------
if [[ "$(id -u)" -eq 0 ]]; then
  fail "refusing to run as root: RustyBGP must never be given a chance to modify the host FIB"
fi
for cmd in git cargo protoc curl python3 ss ip; do
  command -v "$cmd" >/dev/null 2>&1 || fail "required command not found: $cmd"
done
for port in "$BMP_PORT" "$HTTP_PORT" "$R1_BGP_PORT" "$R2_BGP_PORT" "$R1_API_PORT" "$R2_API_PORT"; do
  if port_in_use "$port"; then
    fail "TCP port $port is already in use; stop the previous test / dev server first"
  fi
done
if [[ -n "${http_proxy:-}" || -n "${https_proxy:-}" ]]; then
  pass "using proxy from environment for downloads: ${https_proxy:-$http_proxy}"
fi

rm -rf "$RUN_DIR"
mkdir -p "$RUN_DIR"

# ---------------------------------------------------------------------------
# Build components
# ---------------------------------------------------------------------------
log "building BMP collector (frontend is embedded by build.rs)"
(cd "$ROOT/backend" && cargo build)

log "building RIPE replay tool"
cargo build --manifest-path "$ROOT/tools/ripe-replay/Cargo.toml"

if [[ ! -x "$RUSTYBGP_BIN" ]]; then
  if [[ ! -d "$RUSTYBGP_DIR/.git" ]]; then
    log "cloning RustyBGP into $RUSTYBGP_DIR"
    mkdir -p "$(dirname "$RUSTYBGP_DIR")"
    git clone --depth 1 "$RUSTYBGP_REPO" "$RUSTYBGP_DIR"
  fi
  log "building rustybgpd (this can take several minutes)"
  (cd "$RUSTYBGP_DIR" && cargo build -p rustybgpd)
fi
pass "rustybgpd: $RUSTYBGP_BIN"

# Baseline routing tables immediately before starting any router process.
# (Builds can take minutes; take the snapshot after them to avoid unrelated
# changes from the host's existing BIRD daemon causing noise.)
ip -4 route show table all > "$RUN_DIR/routes-v4.before"
ip -6 route show table all > "$RUN_DIR/routes-v6.before"
pass "snapshotted system RIB (v4: $(wc -l < "$RUN_DIR/routes-v4.before") lines, v6: $(wc -l < "$RUN_DIR/routes-v6.before") lines)"

# ---------------------------------------------------------------------------
# Write configs. NO zebra section anywhere.
# ---------------------------------------------------------------------------
cat > "$RUN_DIR/r1.yaml" <<EOF
# E2E test router r1 (BMP sender). Intentionally NO zebra section:
# kernel FIB writes and next-hop tracking (NHT) stay disabled.
global:
  config:
    as: 65001
    router-id: "10.255.0.1"
    port: $R1_BGP_PORT
    local-address-list:
      - 127.0.0.1
      - "::1"

neighbors:
  - config:
      neighbor-address: "127.0.0.2"
      peer-as: 65002
      description: "rusty-r2"
    transport:
      config:
        local-address: "127.0.0.1"
        remote-port: $R2_BGP_PORT
        passive-mode: true
    afi-safis:
      - config:
          afi-safi-name: ipv4-unicast
  - config:
      neighbor-address: "::1"
      peer-as: 65002
      description: "rusty-r2-ipv6"
    transport:
      config:
        local-address: "::1"
        remote-port: $R2_BGP_PORT
        passive-mode: true
    afi-safis:
      - config:
          afi-safi-name: ipv6-unicast

bmp-servers:
  - config:
      address: "$BMP_CLIENT_HOST"
      port: $BMP_PORT
      route-monitoring-policy: pre-policy
      statistics-timeout: 30
EOF

cat > "$RUN_DIR/r2.yaml" <<EOF
# E2E test router r2 (receives RIPE routes via gRPC, advertises them to r1).
# Intentionally NO zebra section: no kernel FIB writes, no NHT.
global:
  config:
    as: 65002
    router-id: "10.255.0.2"
    port: $R2_BGP_PORT
    local-address-list:
      - 127.0.0.2
      - "::1"

neighbors:
  - config:
      neighbor-address: "127.0.0.1"
      peer-as: 65001
      description: "rusty-r1"
    transport:
      config:
        local-address: "127.0.0.2"
        remote-port: $R1_BGP_PORT
    afi-safis:
      - config:
          afi-safi-name: ipv4-unicast
  - config:
      neighbor-address: "::1"
      peer-as: 65001
      description: "rusty-r1-ipv6"
    transport:
      config:
        local-address: "::1"
        remote-port: $R1_BGP_PORT
    afi-safis:
      - config:
          afi-safi-name: ipv6-unicast
EOF

if grep -Eq '^[[:space:]]*zebra[[:space:]]*:' "$RUN_DIR/r1.yaml" "$RUN_DIR/r2.yaml"; then
  fail "zebra config found: refusing to start (kernel FIB writes would be possible)"
fi
pass "RustyBGP configs contain no zebra section (kernel FIB + NHT disabled)"

# Re-check ports after the (possibly long) builds.
for port in "$BMP_PORT" "$HTTP_PORT" "$R1_BGP_PORT" "$R2_BGP_PORT" "$R1_API_PORT" "$R2_API_PORT"; do
  if port_in_use "$port"; then
    fail "TCP port $port became occupied during the build; aborting"
  fi
done

# ---------------------------------------------------------------------------
# Start BMP collector
# ---------------------------------------------------------------------------
log "starting BMP collector on $BMP_BIND (dashboard $HTTP_PROBE)"
(
  cd "$ROOT/backend"
  exec env ADMIN_PASSWORD="$ADMIN_PASSWORD" ./target/debug/bmp-status \
    --bmp-bind "$BMP_BIND" \
    --http-bind "$HTTP_BIND" \
    --data-dir "$RUN_DIR/bmp-data"
) > "$RUN_DIR/collector.log" 2>&1 &
COLLECTOR_PID=$!

for _ in $(seq 1 100); do
  if curl -sf "$HTTP_PROBE/api/health" >/dev/null 2>&1; then break; fi
  if ! kill -0 "$COLLECTOR_PID" 2>/dev/null; then fail "collector exited during startup"; fi
  sleep 0.2
done
curl -sf "$HTTP_PROBE/api/health" >/dev/null || fail "collector did not become healthy"
pass "collector healthy: $(curl -sf "$HTTP_PROBE/api/health")"

# Register r1 as a managed instance (display name in the dashboard).
curl -sf -c "$RUN_DIR/cookies.txt" -X POST -H 'Content-Type: application/json' \
  -d "{\"password\":\"$ADMIN_PASSWORD\"}" "$HTTP_PROBE/api/admin/login" >/dev/null \
  || fail "admin login failed"
curl -sf -b "$RUN_DIR/cookies.txt" -X POST -H 'Content-Type: application/json' \
  -d '{"name":"rusty-r1","routerIp":"127.0.0.1","description":"RustyBGP AS65001 edge router (BMP pre-policy)","enabled":true}' \
  "$HTTP_PROBE/api/admin/instances" >/dev/null || fail "instance registration failed"
pass "registered BMP instance rusty-r1 (127.0.0.1)"

# ---------------------------------------------------------------------------
# Start the two RustyBGP routers
# ---------------------------------------------------------------------------
log "starting rusty-r1 (AS65001, BMP sender)"
(
  cd "$RUN_DIR"
  exec env RUST_LOG=info "$RUSTYBGP_BIN" -f "$RUN_DIR/r1.yaml" --api-hosts "127.0.0.1:$R1_API_PORT"
) > "$RUN_DIR/r1.log" 2>&1 &
R1_PID=$!

log "starting rusty-r2 (AS65002, RIPE route receiver)"
(
  cd "$RUN_DIR"
  exec env RUST_LOG=info "$RUSTYBGP_BIN" -f "$RUN_DIR/r2.yaml" --api-hosts "127.0.0.1:$R2_API_PORT"
) > "$RUN_DIR/r2.log" 2>&1 &
R2_PID=$!

sleep 1

# Hard runtime guard: no effective CAP_NET_ADMIN in either router process.
for pair in "r1:$R1_PID" "r2:$R2_PID"; do
  name="${pair%%:*}"
  pid="${pair##*:}"
  [[ -r "/proc/$pid/status" ]] || fail "$name process is not running"
  cap_eff="$(awk '/^CapEff:/{print $2}' "/proc/$pid/status")"
  python3 - "$name" "$cap_eff" <<'PY' || fail "$name has CAP_NET_ADMIN or unreadable capabilities"
import sys
name, cap = sys.argv[1], int(sys.argv[2], 16)
if cap & 0x1000:
    print(f"{name} CapEff={cap:016x} includes CAP_NET_ADMIN", file=sys.stderr)
    raise SystemExit(1)
print(f"  PASS  {name} has no CAP_NET_ADMIN (CapEff={cap:016x})")
PY
done

# Wait for BGP Established + BMP PeerUp.
log "waiting for IPv4 + IPv6 BGP sessions and BMP PeerUp"
ready=0
for _ in $(seq 1 200); do
  if curl -sf "$HTTP_PROBE/api/overview" 2>/dev/null | python3 -c '
import json, sys
d = json.load(sys.stdin)
raise SystemExit(0 if d["activeConnections"] >= 1 and d["peersUp"] >= 2 else 1)
' 2>/dev/null; then
    ready=1
    break
  fi
  sleep 0.25
done
[[ "$ready" -eq 1 ]] || fail "BGP session / BMP PeerUp did not appear in time"
pass "IPv4 + IPv6 BGP sessions Established; both BMP PeerUp messages received"

if grep -qi "kernel route integration enabled" "$RUN_DIR/r1.log" "$RUN_DIR/r2.log"; then
  fail "RustyBGP kernel route integration was enabled; STOPPING (this would write the system FIB)"
fi
pass "no kernel route integration in RustyBGP logs"

# ---------------------------------------------------------------------------
# Replay cached RIPE routes into r2's Loc-RIB via gRPC
# ---------------------------------------------------------------------------
log "replaying RIPE RIS routes into rusty-r2 (cache: $RIPE_CACHE)"
RUST_LOG=info "$ROOT/tools/ripe-replay/target/debug/ripe-bgp-replay" \
  --url "$RIPE_URL" \
  --cache-dir "$RIPE_CACHE" \
  --limit "$LIMIT" \
  --limit-v6 "$LIMIT_V6" \
  --api "http://127.0.0.1:$R2_API_PORT" \
  --summary-file "$RUN_DIR/replay.json" \
  | tee "$RUN_DIR/replay.log"

injected="$(python3 -c 'import json;print(json.load(open("'"$RUN_DIR"'/replay.json"))["injected"])')"
target="$injected"
if [[ "$target" -lt "$MIN_ROUTES" ]]; then target="$MIN_ROUTES"; fi
if [[ "$target" -gt "$injected" ]]; then target="$injected"; fi
[[ "$target" -gt 0 ]] || fail "replay injected 0 routes"

log "waiting for BMP Route Monitoring (target >= $target active routes)"
routes=0
for _ in $(seq 1 240); do
  routes="$(curl -sf "$HTTP_PROBE/api/overview" 2>/dev/null | python3 -c 'import json,sys;print(json.load(sys.stdin)["activeRoutes"])' 2>/dev/null || echo 0)"
  if [[ "$routes" -ge "$target" ]]; then break; fi
  sleep 0.25
done
[[ "$routes" -ge "$target" ]] || fail "only $routes routes appeared, expected >= $target"

# ---------------------------------------------------------------------------
# Assertions
# ---------------------------------------------------------------------------
log "validating dashboard data"
python3 - "$HTTP_PROBE" "$RUN_DIR" <<'PY'
import json, pathlib, sys, urllib.request

base, run = sys.argv[1], pathlib.Path(sys.argv[2])

def get(path):
    with urllib.request.urlopen(base + path, timeout=10) as response:
        return json.load(response)

overview = get("/api/overview")

# The route API is explicitly paginated. Fetch all pages for the assertions
# below instead of requesting one oversized snapshot.
routes = get("/api/routes?page=1&limit=200")
all_routes = list(routes["routes"])
for page in range(2, routes.get("totalPages", 1) + 1):
    all_routes.extend(get(f"/api/routes?page={page}&limit=200")["routes"])
routes["routes"] = all_routes

messages = get("/api/messages?type=route_monitoring&limit=100")
summary = json.loads((run / "replay.json").read_text())
injected = summary["injected"]
injected_v4 = summary["injected_v4"]
injected_v6 = summary["injected_v6"]
minimum = min(injected, 100)
minimum_v4 = min(injected_v4, 100)
minimum_v6 = min(injected_v6, 20)
assert injected_v6 > 0, "IPv6 replay injected zero routes"
assert injected_v4 > 0, "IPv4 replay injected zero routes"

assert overview["activeRoutes"] >= minimum, overview["activeRoutes"]
assert len(routes["routes"]) >= minimum, len(routes["routes"])
assert overview["totals"]["parseErrors"] == 0, overview["totals"]

v4_peer = next((p for p in overview["peers"] if p["ip"] == "127.0.0.2" and p["asn"] == 65002), None)
assert v4_peer is not None, overview["peers"]
assert v4_peer["session"] == "up", v4_peer
assert v4_peer["prefixes"] >= minimum_v4, v4_peer

v6_peer = next((p for p in overview["peers"] if p["ip"] == "::1" and p["asn"] == 65002), None)
assert v6_peer is not None, overview["peers"]
assert v6_peer["ipv6"] is True, v6_peer
assert v6_peer["session"] == "up", v6_peer
assert v6_peer["prefixes"] >= minimum_v6, v6_peer

instance = next((i for i in overview["instances"] if i["name"] == "rusty-r1"), None)
assert instance is not None, overview["instances"]
assert instance["online"] is True, instance
assert instance["peers"] >= 2, instance

v4_routes = [r for r in routes["routes"] if ":" not in r["prefix"]]
v6_routes = [r for r in routes["routes"] if ":" in r["prefix"]]
assert len(v4_routes) >= minimum_v4, len(v4_routes)
assert len(v6_routes) >= minimum_v6, len(v6_routes)
assert all(r["peerAsn"] == 65002 for r in v6_routes), "unexpected IPv6 peer"
assert any(r["nextHop"] and ":" in r["nextHop"] for r in v6_routes), "IPv6 routes have no IPv6 next hop"
assert any(r["asPath"] and r["asPath"][0] == 65002 for r in v6_routes), "IPv6 AS paths missing 65002"

sample_prefixes = [s["prefix"] for s in summary["sample"]]
v6_sample_prefixes = [s["prefix"] for s in summary["sample"] if s["family"] == "ipv6"]
rib_prefixes = {r["prefix"] for r in routes["routes"]}
matched = [p for p in sample_prefixes if p in rib_prefixes]
assert any(p in rib_prefixes for p in v6_sample_prefixes), (v6_sample_prefixes[:3], len(rib_prefixes))

as_paths = [r["asPath"] for r in routes["routes"] if r["asPath"]]
assert as_paths, "routes have no AS paths"
assert all(path[0] == 65002 for path in as_paths[:50]), as_paths[:5]
assert any(len(path) > 4 for path in as_paths), as_paths[:5]

assert messages, "no BMP route-monitoring messages"
assert any(
    m["peer"] and m["peer"]["asn"] == 65002 and m["routerAddr"] == "127.0.0.1"
    for m in messages
), messages[0]
assert any(m["peer"] and m["peer"].get("ipv6") for m in messages), "no IPv6 BMP peer in messages"

sample_v6 = next((r for r in v6_routes if r["asPath"]), v6_routes[0])
print(f"  active routes:      {overview['activeRoutes']} (IPv4 {len(v4_routes)} / IPv6 {len(v6_routes)})")
print(f"  BMP messages:       {overview['totals']['messages']} (route monitoring {overview['totals']['routeMonitoring']})")
print(f"  IPv4 peer:          {v4_peer['ip']} AS{v4_peer['asn']} {v4_peer['session']} ({v4_peer['prefixes']} prefixes)")
print(f"  IPv6 peer:          {v6_peer['ip']} AS{v6_peer['asn']} {v6_peer['session']} ({v6_peer['prefixes']} prefixes, ipv6={v6_peer['ipv6']})")
print(f"  BMP instance:       {instance['name']} online={instance['online']} messages={instance['messages']}")
print(f"  matched samples:    {len(matched)} (IPv6 sample present: {any(p in rib_prefixes for p in v6_sample_prefixes)})")
print(f"  sample IPv6 prefix: {sample_v6['prefix']}")
print(f"  sample IPv6 next hop: {sample_v6['nextHop']}")
print(f"  sample IPv6 AS path: {' '.join(map(str, sample_v6['asPath']))}")
print(f"  RIPE cache:         hit={summary['cache_hit']} downloaded={summary['downloaded']} file={summary['cache_path']}")
PY
pass "BMP dashboard contains routes replayed from RIPE RIS"

# ---------------------------------------------------------------------------
# Post-run FIB safety checks
# ---------------------------------------------------------------------------
ip -4 route show table all > "$RUN_DIR/routes-v4.after"
ip -6 route show table all > "$RUN_DIR/routes-v6.after"

# Diffs are written to files; only a short head is ever printed to the
# terminal/AI window. Never dump a full routing table.
if ! diff -u "$RUN_DIR/routes-v4.before" "$RUN_DIR/routes-v4.after" > "$RUN_DIR/routes-v4.diff"; then
  head -60 "$RUN_DIR/routes-v4.diff" >&2 || true
  fail "system IPv4 routing table changed during the test! (full diff: $RUN_DIR/routes-v4.diff)"
fi
if ! diff -u "$RUN_DIR/routes-v6.before" "$RUN_DIR/routes-v6.after" > "$RUN_DIR/routes-v6.diff"; then
  head -60 "$RUN_DIR/routes-v6.diff" >&2 || true
  fail "system IPv6 routing table changed during the test! (full diff: $RUN_DIR/routes-v6.diff)"
fi
pass "system IPv4 and IPv6 route tables are byte-for-byte unchanged"

python3 - "$RUN_DIR/replay.json" > "$RUN_DIR/ripe-prefixes.txt" <<'PY'
import json, sys
summary = json.load(open(sys.argv[1]))
for route in summary["sample"]:
    print(route["prefix"])
PY
if grep -F -f "$RUN_DIR/ripe-prefixes.txt" <(ip -4 route show) >/dev/null 2>&1; then
  fail "a replayed RIPE IPv4 prefix appeared in the kernel routing table"
fi
if grep -F -f "$RUN_DIR/ripe-prefixes.txt" <(ip -6 route show) >/dev/null 2>&1; then
  fail "a replayed RIPE IPv6 prefix appeared in the kernel routing table"
fi
pass "no replayed RIPE IPv4/IPv6 prefix exists in the kernel RIB"

if grep -qi "kernel route integration enabled" "$RUN_DIR/r1.log" "$RUN_DIR/r2.log"; then
  fail "kernel route integration was enabled at some point; STOPPING"
fi
pass "FIB guard: non-root + no zebra config + no CAP_NET_ADMIN + route tables unchanged"

printf '\n\033[1;32m=== E2E PASS ===\033[0m\n'
printf 'BMP:  %s  (dashboard %s/)\n' "$BMP_BIND" "$HTTP_PROBE"
printf 'Logs: %s\n' "$RUN_DIR"
printf 'RIPE cache: %s\n' "$RIPE_CACHE"

if [[ "$KEEP_RUNNING" == "1" ]]; then
  trap - EXIT INT TERM
  printf '\nKEEP_RUNNING=1: leaving services running\n'
  printf '  dashboard: %s/\n' "$HTTP_PROBE"
  printf '  admin:     %s/admin/  password=%s\n' "$HTTP_PROBE" "$ADMIN_PASSWORD"
  printf '  PIDs: collector=%s r1=%s r2=%s\n' "$COLLECTOR_PID" "$R1_PID" "$R2_PID"
  printf '  stop: kill %s %s %s\n' "$COLLECTOR_PID" "$R1_PID" "$R2_PID"
fi
