#!/bin/bash
# Clean docker containers and run command with logging
# Usage: ./cleanrun.sh <test_name> <command and arguments>

[[ $# -lt 2 ]] && { echo "Usage: $0 <test_name> <command and arguments>"; exit 1; }

TEST_NAME="$1"
shift  # Remove test_name from arguments

# Docker cleanup
echo "🧹 Cleaning all containers..."
docker ps -aq | xargs -r docker stop 2>/dev/null
docker ps -aq | xargs -r docker rm 2>/dev/null

# Setup QEMU if needed
if [ "$(uname -m)" = "x86_64" ] && ! grep -q "flags: .*C" /proc/sys/fs/binfmt_misc/qemu-aarch64 2>/dev/null; then
    docker run --rm --privileged multiarch/qemu-user-static --reset -p yes --credential yes
fi

# Setup logging with test name
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BASE_DIR="/tmp/qemu-tests/${TEST_NAME}_${TIMESTAMP}"
mkdir -p "$BASE_DIR"

echo "🚀 Test: $TEST_NAME"
echo "🚀 Running: $*"
echo "📊 Monitor: tail -f $BASE_DIR/full.log"

# Run with timestamped logging
"$@" 2> >(while read line; do
    echo "[$(date '+%H:%M:%S')] $line" | tee -a "$BASE_DIR/stderr.log" "$BASE_DIR/full.log" >&2
done) 1> >(while read line; do
    echo "[$(date '+%H:%M:%S')] $line" | tee -a "$BASE_DIR/stdout.log" "$BASE_DIR/full.log"
done)