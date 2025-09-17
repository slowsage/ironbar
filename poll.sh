#!/bin/bash

# Generic polling script for long-running commands
# Usage: ./poll.sh [options] -- command [command args...]
# Examples:
#   ./poll.sh -- act test -j test -W .github/workflows/test.yml
#   ./poll.sh -i 10 -t 600 -- docker build .
#   ./poll.sh --interval 2 --lines 20 -- cargo build --release
#   ./poll.sh -q -- make all

set -e

# Default values
DEFAULT_INTERVAL=5
DEFAULT_TIMEOUT=300
DEFAULT_LINES=10

# Parse options
POLL_INTERVAL=$DEFAULT_INTERVAL
TIMEOUT=$DEFAULT_TIMEOUT  
SHOW_LINES=$DEFAULT_LINES
QUIET=false

print_usage() {
    cat << EOF
Usage: $0 [options] -- command [command args...]

Options:
    -i, --interval SECONDS    Polling interval (default: $DEFAULT_INTERVAL)
    -t, --timeout SECONDS     Maximum runtime (default: $DEFAULT_TIMEOUT)
    -l, --lines NUMBER        Lines to show in progress (default: $DEFAULT_LINES)
    -q, --quiet              Suppress progress output
    -h, --help               Show this help message

Examples:
    $0 -- act test -j test -W workflow.yml
    $0 -i 10 -t 600 -- docker build .
    $0 --quiet -- make all

The command and its arguments must come after '--'
EOF
    exit 1
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -i|--interval)
            POLL_INTERVAL="$2"
            shift 2
            ;;
        -t|--timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        -l|--lines)
            SHOW_LINES="$2"
            shift 2
            ;;
        -q|--quiet)
            QUIET=true
            shift
            ;;
        -h|--help)
            print_usage
            ;;
        --)
            shift
            break
            ;;
        *)
            echo "Error: Unknown option '$1'"
            echo "Use -- before your command"
            print_usage
            ;;
    esac
done

# Check if command provided
if [[ $# -eq 0 ]]; then
    echo "Error: No command provided"
    print_usage
fi

# Build command from remaining arguments
CMD="$*"
LOG_FILE="/tmp/poll-$(date +%s)-$$.log"

if [[ "$QUIET" != "true" ]]; then
    echo "Poll Script"
    echo "==========="
    echo "Command: $CMD"
    echo "Log file: $LOG_FILE"
    echo "Poll interval: ${POLL_INTERVAL}s"
    echo "Timeout: ${TIMEOUT}s"
    echo "----------------------------------------"
fi

# Run command in background
eval "$CMD" > "$LOG_FILE" 2>&1 &
CMD_PID=$!

if [[ "$QUIET" != "true" ]]; then
    echo "Process started with PID: $CMD_PID"
    echo "----------------------------------------"
fi

# Monitor with timeout
START_TIME=$(date +%s)
ELAPSED=0

while kill -0 "$CMD_PID" 2>/dev/null && [ $ELAPSED -lt $TIMEOUT ]; do
    sleep $POLL_INTERVAL
    
    CURRENT_TIME=$(date +%s)
    ELAPSED=$((CURRENT_TIME - START_TIME))
    
    if [[ "$QUIET" != "true" ]]; then
        echo "[$(date '+%H:%M:%S')] Progress (${ELAPSED}s elapsed):"
        tail -"$SHOW_LINES" "$LOG_FILE" 2>/dev/null || echo "  (no output yet)"
        echo "----------------------------------------"
    fi
done

# Check if timeout occurred
if [ $ELAPSED -ge $TIMEOUT ] && kill -0 "$CMD_PID" 2>/dev/null; then
    echo "⏰ Timeout reached (${TIMEOUT}s), killing process..."
    kill "$CMD_PID" 2>/dev/null || true
    sleep 2
    kill -9 "$CMD_PID" 2>/dev/null || true
    echo "❌ Command timed out after ${TIMEOUT} seconds"
    echo ""
    echo "=== FINAL OUTPUT (TIMEOUT) ==="
    tail -50 "$LOG_FILE" 2>/dev/null || echo "No output available"
    echo "Log file: $LOG_FILE"
    exit 124  # Timeout exit code
fi

# Process finished naturally
wait $CMD_PID 2>/dev/null
EXIT_CODE=$?

if [[ "$QUIET" != "true" ]] || [ $EXIT_CODE -ne 0 ]; then
    echo ""
    echo "=== COMMAND COMPLETED ==="
    echo "Duration: ${ELAPSED}s"
    echo "Exit code: $EXIT_CODE"
    
    if [ $EXIT_CODE -eq 0 ]; then
        echo "✅ Success"
    else
        echo "❌ Failed"
        echo ""
        echo "=== OUTPUT ==="
        cat "$LOG_FILE"
    fi
    echo "Log file: $LOG_FILE"
fi

exit $EXIT_CODE