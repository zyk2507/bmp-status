#!/usr/bin/env bash
# Build amd64/arm64 container images from the static musl binaries, create a
# multi-architecture manifest, and save deployable archives under prebuilt-images/.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/build-env.sh
source "$ROOT/scripts/build-env.sh"

PODMAN="${PODMAN:-podman}"
IMAGE_NAME="${IMAGE_NAME:-bmp-status}"
IMAGE_TAG="${IMAGE_TAG:-latest}"
VERSION="${VERSION:-0.1.0}"
ARCHES="${ARCHES:-amd64 arm64}"
IMAGE_DIR="$ROOT/prebuilt-images"
MANIFEST_ID_FILE="$IMAGE_DIR/.manifest-ids"

if [[ -z "${PODMAN_OPTS:-}" ]]; then
    PODMAN_OPTS=""
    [[ -n "${PODMAN_ROOT:-}" ]] && PODMAN_OPTS+=" --root ${PODMAN_ROOT}"
    [[ -n "${PODMAN_RUNROOT:-}" ]] && PODMAN_OPTS+=" --runroot ${PODMAN_RUNROOT}"
fi

read -r -a PODMAN_ARGS <<< "${PODMAN_OPTS:-}"

for arch in $ARCHES; do
    if [[ ! -f "$ROOT/bin/bmp-status-linux-${arch}-musl" ]]; then
        echo "ERROR: bin/bmp-status-linux-${arch}-musl is missing." >&2
        echo "       Run 'make cross-binaries' first." >&2
        exit 1
    fi
done

mkdir -p "$IMAGE_DIR"
rm -f "$IMAGE_DIR"/bmp-status-"${VERSION}"-linux-*.tar

# Remove manifest lists recorded by a previous run, including lists that were
# turned into dangling entries by load.sh tagging an image as latest.
if [[ -f "$MANIFEST_ID_FILE" ]]; then
    while read -r old_id; do
        [[ -n "$old_id" ]] || continue
        "${PODMAN}" "${PODMAN_ARGS[@]}" manifest rm "$old_id" >/dev/null 2>&1 || \
            "${PODMAN}" "${PODMAN_ARGS[@]}" rmi -f "$old_id" >/dev/null 2>&1 || true
    done < "$MANIFEST_ID_FILE"
    rm -f "$MANIFEST_ID_FILE"
fi

echo "==> Building Alpine runtime images..."
for arch in $ARCHES; do
    platform_tag="localhost/${IMAGE_NAME}:${IMAGE_TAG}-${arch}"
    echo "--> ${platform_tag}"
    "${PODMAN}" "${PODMAN_ARGS[@]}" build \
        --arch "$arch" \
        --format docker \
        --network=host \
        --build-arg "BINARY=bin/bmp-status-linux-${arch}-musl" \
        -t "$platform_tag" \
        -f "$ROOT/container/Dockerfile.runtime" \
        "$ROOT"
done

# Keep a local multi-architecture manifest for running on the build host.
# Container archives are per architecture because Podman cannot load an OCI
# index as a manifest list from a local tar file.
mapfile -t MANIFEST_TAGS < <(printf '%s\n' "$IMAGE_TAG" "latest" | awk '!seen[$0]++')
for tag in "${MANIFEST_TAGS[@]}"; do
    manifest="localhost/${IMAGE_NAME}:${tag}"
    # Replace a stale manifest or image with the same name.
    "${PODMAN}" "${PODMAN_ARGS[@]}" manifest rm "$manifest" >/dev/null 2>&1 || \
        "${PODMAN}" "${PODMAN_ARGS[@]}" rmi -f "$manifest" >/dev/null 2>&1 || true
    "${PODMAN}" "${PODMAN_ARGS[@]}" manifest create "$manifest"
    for arch in $ARCHES; do
        "${PODMAN}" "${PODMAN_ARGS[@]}" manifest add "$manifest" \
            "containers-storage:localhost/${IMAGE_NAME}:${IMAGE_TAG}-${arch}"
    done
    manifest_id="$("${PODMAN}" "${PODMAN_ARGS[@]}" images --format '{{.ID}}' "$manifest" | head -n1)"
    if [[ -n "$manifest_id" ]]; then
        printf '%s\n' "$manifest_id" >> "$MANIFEST_ID_FILE"
    fi
    echo "--> Manifest: $manifest"
done

echo "==> Saving per-architecture archives..."
for arch in $ARCHES; do
    archive="$IMAGE_DIR/bmp-status-${VERSION}-linux-${arch}.tar"
    "${PODMAN}" "${PODMAN_ARGS[@]}" save --format docker-archive -o "$archive" \
        "localhost/${IMAGE_NAME}:${IMAGE_TAG}-${arch}"
    echo "--> $archive"
done

install -m 0755 "$ROOT/scripts/load-prebuilt.sh" "$IMAGE_DIR/load.sh"

(
    cd "$IMAGE_DIR"
    sha256sum ./*.tar ./load.sh > SHA256SUMS
)

# Remove old BMP Status image layers left behind when a tag moved to a new
# image. Tagged/in-use images are not affected by the default prune mode.
"${PODMAN}" "${PODMAN_ARGS[@]}" image prune -f \
    --filter "label=org.opencontainers.image.title=BMP Status" >/dev/null 2>&1 || true

echo "==> Container image artifacts:"
ls -lh "$IMAGE_DIR"
echo
echo "Copy prebuilt-images/ to the target machine and run:"
echo "  cd prebuilt-images && ./load.sh"
echo "Remove local build tags/manifests with:"
echo "  make clean-images"
