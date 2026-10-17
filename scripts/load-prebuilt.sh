#!/usr/bin/env bash
# Load the correct prebuilt image for this machine and tag it as latest.
#
# Copy this directory (prebuilt-images/) to the target machine, then run:
#   ./load.sh
#
# The archives are produced by `make cross-images`.

set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGE_NAME="${IMAGE_NAME:-bmp-status}"
IMAGE_TAG="${IMAGE_TAG:-latest}"
PODMAN="${PODMAN:-podman}"

if [[ -z "${PODMAN_OPTS:-}" ]]; then
    PODMAN_OPTS=""
    [[ -n "${PODMAN_ROOT:-}" ]] && PODMAN_OPTS+=" --root ${PODMAN_ROOT}"
    [[ -n "${PODMAN_RUNROOT:-}" ]] && PODMAN_OPTS+=" --runroot ${PODMAN_RUNROOT}"
fi
read -r -a PODMAN_ARGS <<< "${PODMAN_OPTS:-}"

case "$(uname -m)" in
    x86_64|amd64)
        ARCH="amd64"
        ;;
    aarch64|arm64)
        ARCH="arm64"
        ;;
    *)
        echo "ERROR: unsupported machine architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

shopt -s nullglob
archives=("$DIR/${IMAGE_NAME}-"*-linux-"${ARCH}".tar)
if [[ ${#archives[@]} -eq 0 ]]; then
    echo "ERROR: no ${ARCH} image archive found in $DIR" >&2
    exit 1
fi

# Pick the newest archive if several versions are present.
archive=""
archive_mtime=0
for candidate in "${archives[@]}"; do
    mtime="$(stat -c %Y "$candidate")"
    if (( mtime > archive_mtime )); then
        archive="$candidate"
        archive_mtime="$mtime"
    fi
done

echo "==> Loading $archive"
"${PODMAN}" "${PODMAN_ARGS[@]}" load -i "$archive"

source_tag="localhost/${IMAGE_NAME}:${IMAGE_TAG}-${ARCH}"
if [[ "$IMAGE_TAG" == "latest" ]]; then
    source_tag="localhost/${IMAGE_NAME}:latest-${ARCH}"
fi

if ! "${PODMAN}" "${PODMAN_ARGS[@]}" image exists "$source_tag"; then
    echo "ERROR: loaded image tag not found: $source_tag" >&2
    echo "       Run 'podman images' and tag the image manually." >&2
    exit 1
fi

# If this host previously had the multi-architecture manifest, remove it
# explicitly so that tagging the local image does not leave a dangling list.
if "${PODMAN}" "${PODMAN_ARGS[@]}" manifest exists "localhost/${IMAGE_NAME}:latest"; then
    "${PODMAN}" "${PODMAN_ARGS[@]}" manifest rm "localhost/${IMAGE_NAME}:latest" >/dev/null 2>&1 || true
fi

"${PODMAN}" "${PODMAN_ARGS[@]}" tag "$source_tag" "localhost/${IMAGE_NAME}:latest"
echo "==> Ready: localhost/${IMAGE_NAME}:latest"
echo "    Run: podman run -d --name bmp-status -p 8787:8787 \\"
echo "           -v bmp-status-data:/var/lib/bmp-status localhost/${IMAGE_NAME}:latest"
