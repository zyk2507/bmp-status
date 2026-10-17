#!/usr/bin/env bash
# Remove the local image tags and manifest lists created by cross-image.sh.
# Prebuilt archives under prebuilt-images/ are left untouched.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PODMAN="${PODMAN:-podman}"
IMAGE_NAME="${IMAGE_NAME:-bmp-status}"
IMAGE_TAG="${IMAGE_TAG:-latest}"
ARCHES="${ARCHES:-amd64 arm64}"
IMAGE_DIR="$ROOT/prebuilt-images"
MANIFEST_ID_FILE="$IMAGE_DIR/.manifest-ids"

if [[ -z "${PODMAN_OPTS:-}" ]]; then
    PODMAN_OPTS=""
    [[ -n "${PODMAN_ROOT:-}" ]] && PODMAN_OPTS+=" --root ${PODMAN_ROOT}"
    [[ -n "${PODMAN_RUNROOT:-}" ]] && PODMAN_OPTS+=" --runroot ${PODMAN_RUNROOT}"
fi

read -r -a PODMAN_ARGS <<< "${PODMAN_OPTS:-}"

if [[ -f "$MANIFEST_ID_FILE" ]]; then
    while read -r manifest_id; do
        [[ -n "$manifest_id" ]] || continue
        "${PODMAN}" "${PODMAN_ARGS[@]}" manifest rm "$manifest_id" >/dev/null 2>&1 || \
            "${PODMAN}" "${PODMAN_ARGS[@]}" rmi -f "$manifest_id" >/dev/null 2>&1 || true
    done < "$MANIFEST_ID_FILE"
    rm -f "$MANIFEST_ID_FILE"
fi

mapfile -t REMOVE_TAGS < <(printf '%s\n' "$IMAGE_TAG" latest | awk '!seen[$0]++')

for tag in "${REMOVE_TAGS[@]}"; do
    ref="localhost/${IMAGE_NAME}:${tag}"
    "${PODMAN}" "${PODMAN_ARGS[@]}" manifest rm "$ref" >/dev/null 2>&1 || true
    # The loader tags a plain image as latest, so remove that tag as well.
    "${PODMAN}" "${PODMAN_ARGS[@]}" rmi -f "$ref" >/dev/null 2>&1 || true
done

for tag in "${REMOVE_TAGS[@]}"; do
    for arch in $ARCHES; do
        ref="localhost/${IMAGE_NAME}:${tag}-${arch}"
        "${PODMAN}" "${PODMAN_ARGS[@]}" rmi -f "$ref" >/dev/null 2>&1 || true
    done
done

"${PODMAN}" "${PODMAN_ARGS[@]}" image prune -f \
    --filter "label=org.opencontainers.image.title=BMP Status" >/dev/null 2>&1 || true

# Podman's image prune does not remove dangling manifest lists. Remove any
# remaining dangling entries in the selected storage explicitly.
mapfile -t DANGLING < <("${PODMAN}" "${PODMAN_ARGS[@]}" images --filter dangling=true --format '{{.ID}}')
for dangling_id in "${DANGLING[@]}"; do
    [[ -n "$dangling_id" ]] || continue
    "${PODMAN}" "${PODMAN_ARGS[@]}" rmi -f "$dangling_id" >/dev/null 2>&1 || \
        "${PODMAN}" "${PODMAN_ARGS[@]}" manifest rm "$dangling_id" >/dev/null 2>&1 || true
done

echo "==> Removed local ${IMAGE_NAME} image tags and manifests."
