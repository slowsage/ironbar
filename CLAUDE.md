# CLAUDE.md - ARM64 Build Optimization

## Performance Results
**Target**: ARM64 binary build via QEMU emulation
**Baseline**: 40+ minutes without optimizations
**Optimized**: 8m17s with minimal set, 7m40s with all optimizations
**Speedup achieved**: 5x improvement through systematic QEMU and APT optimizations

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

## Auto-Detection Logic
```yaml
if pmap 1 2>/dev/null | grep -q qemu; then
  echo "QEMU_CPU=max,pauth=off" >> $GITHUB_ENV
  echo "CARGO_BUILD_JOBS=1" >> $GITHUB_ENV
  echo "RUSTFLAGS=-C codegen-units=1 -C opt-level=0 -C lto=false -C debuginfo=0 -C strip=symbols" >> $GITHUB_ENV
fi
```

## Testing & Verification

### Quick Build Test
```bash
./cleanrun.sh testname act -W .github/workflows/binary.yml workflow_dispatch
# Monitor: tail -f /tmp/qemu-tests/testname_*/full.log
```

### Verify ARM64 Binary
```bash
docker exec $(docker ps -qf name=act | head -1) file target/aarch64-unknown-linux-gnu/release/ironbar
# Expected: ELF 64-bit LSB pie executable, ARM aarch64, version 1 (SYSV)
```

### Check QEMU Detection
```bash
docker exec $(docker ps -qf name=act | head -1) pmap 1 | grep qemu
# Should show qemu processes if emulation is active
```

## Implementation Notes

### Current Configuration (Minimal Set)
The repository uses optimizations 1-9 for 8m17s builds:
- QEMU optimizations in `.github/workflows/binary.yml`
- Essential APT optimizations in `.github/scripts/ubuntu_setup.sh`

### Optional Environment Variables
Optimizations 10-15 can be added to `ubuntu_setup.sh` for additional 37-second savings.

### Architecture
- **Before**: 40+ minute builds with no optimizations
- **After**: 8-10 minute builds with systematic optimizations
- **Benefits**: 5x speedup through QEMU pointer authentication bypass and APT hook elimination

## Troubleshooting

### If build is slow (>15 minutes)
1. Check QEMU detection: `pmap 1 | grep qemu`
2. Verify QEMU_CPU is set: `echo $QEMU_CPU`
3. Ensure CARGO_BUILD_JOBS=1: `echo $CARGO_BUILD_JOBS`
4. Monitor CPU usage: `ps aux --sort=-%cpu | head -10`

### If cargo build fails with SIGSEGV
1. CARGO_BUILD_JOBS should be 1: `echo $CARGO_BUILD_JOBS`
2. Parallel builds cause memory exhaustion crashes under QEMU emulation

### Critical: Optimization Dependencies
- QEMU_CPU=max,pauth=off is the primary performance fix (17x improvement)
- CARGO_BUILD_JOBS=1 prevents crashes and must be used with QEMU_CPU
- APT optimizations prevent CPU-intensive hooks during dependency installation
- All three QEMU optimizations (1-3) work synergistically and should be used together