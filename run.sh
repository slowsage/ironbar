#!/usr/bin/env bash
# Enable QEMU credential support for setuid binaries (fixes sudo in ARM64 containers)
docker run --rm --privileged multiarch/qemu-user-static --reset -p yes --credential yes >/dev/null 2>&1
docker stop $(docker ps -aq) 2>/dev/null
docker system prune --volumes -f
time act -W .github/workflows/binary.yml --matrix os:ubuntu-24.04-arm workflow_dispatch -r 2>&1 | ts '[%H:%M:%S]' | tee /tmp/ironbar_build_$(git branch --show-current)_$(date +%Y%m%d_%H%M%S).log

# TO run inside:
# Discover system dependencies using cargo metadata (works without building):
# Method 1: Get crates with 'links' field (direct library specification)
# cargo metadata --format-version=1 | jq -r '.packages[] | select(.links) | .name + " → " + .links'
# Method 2: Get crates using system-deps (pkg-config based)
# cargo metadata --format-version=1 | jq -r '.packages[] | select(.metadata["system-deps"]) | .name + " → system-deps"'
# Combined: Get all system dependencies
# { cargo metadata --format-version=1 | jq -r '.packages[] | select(.links) | .name + " " + .links'; cargo metadata --format-version=1 | jq -r '.packages[] | select(.metadata["system-deps"]) | .name + " system-deps"'; } | sort
