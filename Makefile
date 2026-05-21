.DEFAULT_GOAL := help

DOCKER := $(shell command -v docker 2>/dev/null)
PODMAN := $(shell command -v podman 2>/dev/null)
ifdef DOCKER
  RUNTIME := docker
  COMPOSE  := docker compose
else ifdef PODMAN
  RUNTIME := podman
  COMPOSE  := podman compose
else
  $(error Neither docker nor podman found in PATH)
endif

GHCR_OWNER ?= slmingol
GHCR_IMAGE  := ghcr.io/$(GHCR_OWNER)/toktrack:latest

.PHONY: check fmt clippy test build release setup docker-build docker-pull docker-run docker-report help

help:
	@echo "Usage: make <target>"
	@echo ""
	@echo "Dev"
	@echo "  check        fmt + clippy + test (pre-commit)"
	@echo "  fmt          Format code"
	@echo "  clippy       Lint (warnings = errors)"
	@echo "  test         Run tests"
	@echo "  build        Build debug binary"
	@echo "  release      Build release binary"
	@echo "  setup        Configure git hooks"
	@echo ""
	@echo "Container (docker or podman, auto-detected)"
	@echo "  docker-build  Build image locally"
	@echo "  docker-pull   Pull image from GHCR (ghcr.io/$(GHCR_OWNER)/toktrack)"
	@echo "  docker-run    Launch TUI (interactive)"
	@echo "  docker-report Run CLI report mode"

# Run all checks (used by pre-commit)
check: fmt-check clippy test

# Format code
fmt:
	cargo fmt --all

# Check formatting without modifying
fmt-check:
	cargo fmt --all -- --check

# Run clippy
clippy:
	cargo clippy --all-targets --all-features -- -D warnings

# Run tests
test:
	cargo test

# Build debug
build:
	cargo build

# Build release
release:
	cargo build --release

# Docker/Podman: build image locally
docker-build:
	$(COMPOSE) build

# Pull image from GHCR and tag as toktrack:local for use with docker-run
docker-pull:
	$(RUNTIME) pull $(GHCR_IMAGE)
	$(RUNTIME) tag $(GHCR_IMAGE) toktrack:local

# Docker/Podman: run TUI (interactive)
docker-run:
	$(COMPOSE) run --rm toktrack

# Docker/Podman: run CLI report (non-interactive)
docker-report:
	$(COMPOSE) run --rm toktrack report

# Setup git hooks
setup:
	git config core.hooksPath .githooks
	@echo "Git hooks configured!"
