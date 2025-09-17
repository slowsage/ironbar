#!/bin/bash

echo "=== Cleaning up old Act containers ==="
docker ps -a --format "{{.Names}}" | grep act | xargs -r docker stop
docker ps -a --format "{{.Names}}" | grep act | xargs -r docker rm
echo "✓ Old containers cleaned up"

echo ""
echo "=== Running binary workflow with 30m timeout ==="
./poll.sh -t 1800 -- act -W .github/workflows/binary.yml workflow_dispatch