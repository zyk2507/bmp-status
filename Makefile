SHELL := /bin/bash
.DEFAULT_GOAL := help
.NOTPARALLEL:

ROOT := $(patsubst %/,%,$(abspath $(dir $(lastword $(MAKEFILE_LIST)))))

ARGS ?=
PODMAN ?= podman
PODMAN_ROOT ?=
PODMAN_RUNROOT ?=
PODMAN_OPTS := $(strip $(if $(PODMAN_ROOT),--root $(PODMAN_ROOT)) $(if $(PODMAN_RUNROOT),--runroot $(PODMAN_RUNROOT)))

VERSION ?= $(shell sed -n 's/^version = "\([^"]*\)"/\1/p' backend/Cargo.toml | head -n1)
IMAGE_NAME ?= bmp-status
IMAGE_TAG ?= latest
ARCHES ?= amd64 arm64

export PODMAN
export PODMAN_OPTS
export VERSION
export IMAGE_NAME
export IMAGE_TAG
export ARCHES

.PHONY: help build release run dev frontend frontend-force backend backend-force \
	single-binary cross-binaries cross-images all bundle images typecheck test e2e \
	e2e-keep clean-build clean clean-images print-config admin-password

help:
	@printf '%s\n' \
		'BMP Status build targets:' \
		'' \
		'  make frontend          Build the Next.js frontend (frontend/out)' \
		'  make frontend-force    Remove caches and rebuild the frontend' \
		'  make backend           Build the backend natively and copy it to bin/' \
		'  make backend-force     Clean the backend package and rebuild it' \
		'  make single-binary     Build frontend + backend into one native binary' \
		'  make cross-binaries    Cross-build static musl binaries for amd64 and arm64' \
		'  make cross-images      Build Alpine images for both arches, save archives' \
		'  make all               Run single-binary, cross-binaries, and cross-images' \
		'' \
		'  make images            Alias for cross-images' \
		'  make bundle            Alias for single-binary' \
		'  make run               Run the release binary with the local data directory' \
		'  make dev               Start the frontend and backend development servers' \
		'  make test              Run backend tests and frontend type checking' \
		'  make e2e               Run the RustyBGP end-to-end test' \
		'  make print-config      Print build configuration' \
		'  make clean-images      Remove local image tags/manifests created by this Makefile' \
		'  make clean-build       Remove frontend/backend build outputs' \
		'  make clean             Remove all build outputs, artifacts, and local images' \
		'' \
		'Container builds use Alpine/musl and the CERNET mirror:' \
		'  https://help.mirrors.cernet.edu.cn/debian/' \
		'' \
		'China network handling:' \
		'  The container scripts detect mainland China and prompt for a proxy.' \
		'  PROXY=http://host:port make cross-images' \
		'  FORCE_CN=1 or FORCE_CN=0 overrides detection.' \
		'  ALLOW_NO_PROXY=1 skips the proxy requirement.' \
		'' \
		'Useful variables:' \
		'  VERSION=$(VERSION) IMAGE_NAME=$(IMAGE_NAME) IMAGE_TAG=$(IMAGE_TAG) ARCHES="$(ARCHES)"' \
		'  PODMAN_ROOT=/path PODMAN_RUNROOT=/path  use an isolated Podman storage' \
		'  ARGS="--http-bind 0.0.0.0:8787"                      extra arguments for run/dev'

build:
	cd $(ROOT)/backend && cargo build

release:
	cd $(ROOT)/backend && cargo build --release

run: release
	cd $(ROOT)/backend && ./target/release/bmp-status --data-dir $(ROOT)/data $(ARGS)

dev:
	$(ROOT)/scripts/dev.sh $(ARGS)

frontend:
	@$(ROOT)/scripts/build-frontend.sh

frontend-force:
	@$(ROOT)/scripts/build-frontend.sh --force

backend: frontend
	@$(ROOT)/scripts/build-backend.sh

backend-force: frontend
	@$(ROOT)/scripts/build-backend.sh --force

single-binary: backend
	@echo "==> Single native binary is ready in $(ROOT)/bin"

cross-binaries: frontend
	@$(ROOT)/scripts/cross-binary.sh

cross-images: cross-binaries
	@$(ROOT)/scripts/cross-image.sh

all: single-binary cross-binaries cross-images
	@echo "==> All binaries are in $(ROOT)/bin"
	@echo "==> All container archives are in $(ROOT)/prebuilt-images"

bundle: single-binary
images: cross-images

typecheck:
	cd $(ROOT)/frontend && npm run typecheck

test:
	cd $(ROOT)/backend && cargo test
	cd $(ROOT)/frontend && npm run typecheck

e2e:
	$(ROOT)/tests/e2e-rustybgp/run.sh

e2e-keep:
	KEEP_RUNNING=1 $(ROOT)/tests/e2e-rustybgp/run.sh

clean-build:
	cd $(ROOT)/backend && cargo clean
	rm -rf $(ROOT)/frontend/.next $(ROOT)/frontend/out

clean-images:
	@$(ROOT)/scripts/clean-images.sh

clean: clean-build clean-images
	rm -rf $(ROOT)/bin $(ROOT)/prebuilt-images $(ROOT)/.build.env

print-config:
	@printf '%s\n' \
		'ROOT=$(ROOT)' \
		'VERSION=$(VERSION)' \
		'IMAGE_NAME=$(IMAGE_NAME)' \
		'IMAGE_TAG=$(IMAGE_TAG)' \
		'ARCHES=$(ARCHES)' \
		'PODMAN=$(PODMAN)' \
		'PODMAN_OPTS=$(PODMAN_OPTS)' \
		'Rust targets: x86_64-unknown-linux-musl aarch64-unknown-linux-musl' \
		'Mirror help: https://help.mirrors.cernet.edu.cn/debian/'

admin-password:
	@cat $(ROOT)/data/initial-admin-password.txt 2>/dev/null || \
		echo "Initial admin password not found; it may have been changed through the admin panel or ADMIN_PASSWORD."
