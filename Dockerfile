# Standalone, single-architecture container build for BMP Status.
#
# The Makefile cross-image target is preferred when you need amd64 and arm64
# images from one machine: it reuses the static binaries from bin/ and the
# runtime-only container/Dockerfile.runtime (no qemu required).
#
# This file builds the frontend and backend from source, uses musl for the
# Rust binary, and uses Alpine for the runtime image.
#
# Mirrors default to the CERNET (China Education and Research Network) mirror.
# Debian apt help: https://help.mirrors.cernet.edu.cn/debian/
# Alpine apk help: https://help.mirrors.cernet.edu.cn/alpine/
#
# Build with a proxy and a Docker-format image (keeps HEALTHCHECK):
#   podman build --format docker -t localhost/bmp-status:latest \
#     --build-arg HTTP_PROXY=http://192.168.1.114:7890 \
#     --build-arg HTTPS_PROXY=http://192.168.1.114:7890 \
#     --build-arg NO_PROXY=localhost,127.0.0.1 .

ARG ALPINE_MIRROR=mirrors.cernet.edu.cn
ARG DEBIAN_MIRROR=mirrors.cernet.edu.cn
ARG DEBIAN_SECURITY_MIRROR=mirrors.cernet.edu.cn

# ---------------------------------------------------------------------------
# Frontend
# ---------------------------------------------------------------------------
FROM node:22-bookworm-slim AS frontend-builder

ARG DEBIAN_MIRROR
ARG DEBIAN_SECURITY_MIRROR
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY

ENV HTTP_PROXY=${HTTP_PROXY} \
    HTTPS_PROXY=${HTTPS_PROXY} \
    NO_PROXY=${NO_PROXY} \
    http_proxy=${HTTP_PROXY} \
    https_proxy=${HTTPS_PROXY} \
    no_proxy=${NO_PROXY} \
    NEXT_TELEMETRY_DISABLED=1

# Switch Debian sources to CERNET. The frontend stage does not install apt
# packages, but this keeps container builds on the domestic mirror whenever a
# future package is added.
RUN set -eux; \
    for file in /etc/apt/sources.list /etc/apt/sources.list.d/debian.sources; do \
        if [ -f "$file" ]; then \
            sed -i "s|deb.debian.org|${DEBIAN_MIRROR}|g; s|security.debian.org|${DEBIAN_SECURITY_MIRROR}|g" "$file"; \
        fi; \
    done

WORKDIR /app/frontend

COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund

COPY frontend/ ./
RUN npm run build

# ---------------------------------------------------------------------------
# Rust collector (frontend assets are embedded into the binary)
# ---------------------------------------------------------------------------
FROM rust:1.94-alpine AS backend-builder

ARG ALPINE_MIRROR
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY

ENV HTTP_PROXY=${HTTP_PROXY} \
    HTTPS_PROXY=${HTTPS_PROXY} \
    NO_PROXY=${NO_PROXY} \
    http_proxy=${HTTP_PROXY} \
    https_proxy=${HTTPS_PROXY} \
    no_proxy=${NO_PROXY} \
    CARGO_NET_RETRY=5 \
    CARGO_TERM_COLOR=always

WORKDIR /app

# Switch Alpine repositories to CERNET (hostname only; paths are preserved).
RUN set -eux; \
    sed -i "s|dl-cdn.alpinelinux.org|${ALPINE_MIRROR}|g" /etc/apk/repositories; \
    apk add --no-cache build-base ca-certificates

COPY backend/Cargo.toml backend/Cargo.lock ./backend/
COPY backend/build.rs ./backend/
COPY backend/src ./backend/src
COPY --from=frontend-builder /app/frontend/out ./frontend/out

# build.rs must not invoke npm inside the Rust builder stage.
ENV BMP_SKIP_FRONTEND=1

RUN cargo build --release --locked --manifest-path backend/Cargo.toml

# ---------------------------------------------------------------------------
# Runtime
# ---------------------------------------------------------------------------
FROM alpine:3.22 AS runtime

LABEL org.opencontainers.image.title="BMP Status" \
      org.opencontainers.image.description="BMP (BGP Monitoring Protocol) collector and live dashboard" \
      org.opencontainers.image.base.name="alpine:3.22"

COPY --from=backend-builder /app/backend/target/release/bmp-status /usr/local/bin/bmp-status
COPY --chown=10001:10001 container/data/.keep /var/lib/bmp-status/.keep

ENV DATA_DIR=/var/lib/bmp-status \
    BMP_BIND=0.0.0.0:11019 \
    HTTP_BIND=0.0.0.0:8787 \
    RUST_LOG=bmp_status=info

VOLUME ["/var/lib/bmp-status"]
EXPOSE 11019/tcp 8787/tcp
STOPSIGNAL SIGTERM

USER 10001:10001

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1:8787/api/health || exit 1

ENTRYPOINT ["/usr/local/bin/bmp-status"]
