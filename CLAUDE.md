# CLAUDE.md - ARM64 Build Optimization via QEMU

## Purpose
Enables ARM64 binary builds via QEMU emulation with 5x performance improvement:
- **Before**: 40+ minutes (unoptimized QEMU)
- **After**: 8-15 minutes (optimized QEMU + APT)

## Changes from Master

### binary.yml
- **Removed**: Docker container approach (`ghcr.io/jakestanger/ironbar-build:master`)
- **Added**: Matrix build for native runners
  - `x86_64-unknown-linux-gnu` on `ubuntu-24.04`
  - `aarch64-unknown-linux-gnu` on `ubuntu-24.04-arm`
- **Simplified**: 65 lines → 38 lines
- **Uses**: `taiki-e/upload-rust-binary-action` for cross-compilation

### ubuntu_setup.sh
- **Added**: QEMU detection via `pmap 1 | grep qemu`
- **Added**: QEMU optimizations when emulation detected
- **Added**: APT background process disabling
- **Removed**: GitHub CLI installation
- **Requires**: sudo (no fallback logic)

### .actrc (new)
- **Added**: Platform mappings for local testing
- **Maps**: `ubuntu-24.04-arm` to ARM64 runner image
- **Config**: `--rm=false` keeps containers for debugging

## Complete Optimization Catalog

### QEMU Optimizations (Critical - Must Have)
1. **QEMU_CPU=max,pauth=off** - Disables pointer authentication (main 17x bottleneck)
2. **CARGO_BUILD_JOBS=1** - Prevents SIGSEGV crashes in parallel linking under emulation
3. **RUSTFLAGS=-C codegen-units=1 -C opt-level=0 -C lto=false -C debuginfo=0 -C strip=symbols** - Reduces compilation overhead

### APT Optimizations (Essential for <10 minute builds)
4. **Physical apt-check removal** - Move and replace with dummy script (prevents apt-check processes)
5. **Remove command-not-found** - `rm -f /etc/apt/apt.conf.d/50command-not-found` (prevents 91.5% CPU cnf-update-db)
6. **Disable update-motd** - chmod -x and remove update notification scripts (prevents 100% CPU update-notifier)
7. **DPkg::Post-Invoke=""** for apt-get install - Prevents deb-systemd-helper (95.6% CPU during install)
8. **APT::Update::Post-Invoke=""** for apt-get update - Prevents update hooks
9. **APT::Update::Post-Invoke-Success=""** for both commands - Prevents success hooks

### Environment Variables (Optional - saves ~37 seconds)
10. **DEBIAN_FRONTEND=noninteractive** - Prevent debconf interactive prompts
11. **DEBIAN_PRIORITY=critical** - Only show critical configuration prompts
12. **DEBCONF_NONINTERACTIVE_SEEN=true** - Mark debconf questions as already seen
13. **APT_LISTCHANGES_FRONTEND=none** - Disable package changelog display
14. **MOTD_SHOWN=true** - Prevent message-of-the-day update checks
15. **NEEDRESTART_MODE=a** - Automatically restart services without prompting

## Performance Impact
- **Optimizations 1-3 (QEMU)**: Critical foundation - build fails without these
- **Optimizations 4-9 (APT)**: Essential for <10 minute target - saves ~30 minutes
- **Optimizations 10-15 (Environment)**: Optional refinement - saves ~37 seconds

## Implementation Details

### QEMU Detection Logic
```bash
if pmap 1 2>/dev/null | grep -q qemu; then
  echo "QEMU_CPU=max,pauth=off" >> $GITHUB_ENV
  echo "CARGO_BUILD_JOBS=1" >> $GITHUB_ENV
  echo "RUSTFLAGS=-C codegen-units=1 -C opt-level=0 -C lto=false -C debuginfo=0 -C strip=symbols" >> $GITHUB_ENV
fi
```

### Current Implementation
Uses optimizations 1-9 for 8-15 minute builds:
- **Lines 4-9**: QEMU optimizations in ubuntu_setup.sh
- **Lines 11-14**: APT optimizations in ubuntu_setup.sh
- **Lines 17-18**: APT commands with disabled hooks

## Testing Commands

### Local Testing with Act
```bash
# Test both architectures
act -W .github/workflows/binary.yml workflow_dispatch --privileged

# Test ARM64 only
act -W .github/workflows/binary.yml --matrix os:ubuntu-24.04-arm workflow_dispatch --privileged

# Container cleanup
docker ps -aq --filter name=act | xargs -r docker rm -f; docker volume ls -q | grep '^act-' | xargs -r docker volume rm
```

### Verify ARM64 Binary
```bash
docker exec "$(docker ps -qf name=act | head -1)" file target/aarch64-unknown-linux-gnu/release/ironbar
# Expected: ELF 64-bit LSB pie executable, ARM aarch64, version 1 (SYSV)
```

### Check QEMU Detection
```bash
docker exec "$(docker ps -qf name=act | head -1)" pmap 1 | grep qemu
# Should show qemu processes if emulation is active
```

## Rationale

### Why Each Optimization Group

#### QEMU Optimizations (1-3)
- **QEMU_CPU=max,pauth=off**: ARM64 pointer authentication adds 17x overhead under emulation
- **CARGO_BUILD_JOBS=1**: Parallel linking exhausts memory in QEMU, causes SIGSEGV
- **RUSTFLAGS**: Single codegen unit + no optimization reduces emulation complexity

#### APT Optimizations (4-9)
- **apt-check removal**: Prevents background CPU usage during dependency install
- **command-not-found removal**: Eliminates cnf-update-db consuming 91.5% CPU
- **update-motd disable**: Stops update-notifier consuming 100% CPU
- **Hook disabling**: Prevents deb-systemd-helper consuming 95.6% CPU

#### Why This Approach
- **Native runners**: Simpler than Docker-in-Docker, better act compatibility
- **Matrix builds**: Clear separation of x86_64 vs ARM64 targets
- **taiki-e action**: Handles cross-compilation complexity automatically

## Performance Results
- **x86_64**: Native speed (~2 minutes)
- **ARM64 via QEMU**: 8-15 minutes (5x improvement from 40+ minutes)
- **Total speedup**: 5x through systematic QEMU and APT optimizations

## Known Limitations
- **Requires sudo**: No fallback logic, needs --privileged for act
- **ARM64 still slow**: 8-15 min vs native ~2 min (QEMU overhead)
- **Act dependency**: Local testing requires act with specific runner images

## Dependencies
- **libssl-dev libgtk-3-dev libgtk-layer-shell-dev libinput-dev libdbusmenu-gtk3-dev libdbus-1-dev libpulse-dev libluajit-5.1-dev**
- **act**: For local GitHub Actions testing
- **Docker**: For act container execution