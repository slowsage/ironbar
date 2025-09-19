# QEMU Optimization Performance Testing

**Purpose**: Track timing impact of each QEMU optimization by selectively disabling them.
**Baseline**: All optimizations enabled = ~8 minutes total build time

## Test Results Table

| Test ID | Optimization | Description | Time Without | Time With | Impact | Notes |
|---------|-------------|-------------|--------------|-----------|--------|-------|
| 0 | **BASELINE** | All optimizations enabled | - | 8m00s | - | Target performance |
| 1 | DEBIAN_FRONTEND | noninteractive | | | | Prevents debconf prompts |
| 2 | DEBIAN_PRIORITY | critical | | | | Only critical config prompts |
| 3 | APT_LISTCHANGES | none | | | | Disable changelog display |
| 4 | MOTD_SHOWN | true | | | | Prevent MOTD update checks |
| 5 | apt-check removal | rm 50command-not-found | | | | Remove CPU-intensive update checks |
| 6 | apt-check disable | mv to .disabled | | | | Backup disable method |
| 7 | apt-check stub | echo stub script | | | | Create no-op replacement |
| 8 | dpkg configure | --configure -a | | | | Fix broken packages |
| 9 | APT Post-Invoke | Update hooks disabled | | | | Disable update post-hooks |
| 10 | APT Success hooks | Success hooks disabled | | | | Disable success hooks |
| 11 | DPkg Post-Invoke | Post-install hooks disabled | | | | Disable dpkg post-install hooks |
| 12 | force-confold | Keep old configs | | | | Auto-keep old config files |
| 13 | force-confdef | Use config defaults | | | | Auto-use defaults for new configs |
| 14 | no-install-recommends | Required packages only | | | | Only install required packages |
| 15 | CARGO_BUILD_JOBS=1 | Single build job | | | | **CRITICAL**: Prevent SIGSEGV crashes |
| 16 | QEMU_CPU=max,pauth=off | Disable pointer auth | | | | **CRITICAL**: Main 17x bottleneck |
| 17 | codegen-units=1 | Single codegen unit | | | | Reduce parallelism |
| 18 | opt-level=0 | No optimizations | | | | Faster compilation |
| 19 | lto=false | Disable LTO | | | | Skip link-time optimization |
| 20 | debuginfo=0 | No debug info | | | | Reduce binary size work |
| 21 | strip=symbols | Strip symbols | | | | Reduce final binary size |
| 22 | QEMU detection | pmap conditional | | | | Only apply when needed |

## Testing Method

1. **Test Command**: `./cleanrun.sh act -W .github/workflows/binary.yml workflow_dispatch`
2. **Monitor**: `tail -f /tmp/cleanrun/full_*.log`
3. **For each test**: Comment out one optimization, run test, record time
4. **Time breakdown**:
   - System dependencies: ~4m13s (baseline)
   - Cargo build: ~2m50s (baseline)
   - Setup/checkout: <1 minute

## Critical Optimizations (Expected High Impact)
- **Test 15**: CARGO_BUILD_JOBS=1 (prevents crashes)
- **Test 16**: QEMU_CPU=max,pauth=off (17x speedup documented)
- **Test 5-7**: apt-check removal (CPU-intensive operations)

## Testing Priority Order
1. Test 16 (QEMU_CPU) - Expected largest impact
2. Test 15 (CARGO_BUILD_JOBS) - Prevents failure
3. Tests 5-7 (apt-check) - Known CPU bottleneck
4. Tests 9-11 (hook disabling) - APT performance
5. Remaining optimizations

## Automated Testing Methodology

### Unmonitored Testing Protocol
1. **Launch**: `./test-optimization-groups.sh`
2. **Monitoring**: Script sleeps 2m between checks, auto-monitors progress
3. **Timeout**: Auto-fail any test >20 minutes (unacceptable performance)
4. **Results**: Automatically logged to this file with timestamps

### Test Commands
- **Start test**: `./cleanrun.sh act -W .github/workflows/binary.yml workflow_dispatch`
- **Check progress**: `docker ps -qf name=act | head -1` (get container ID)
- **Monitor logs**: `docker logs -f <container_id>`
- **Verify QEMU**: `docker exec <container_id> pmap 1 | grep qemu`

### Success Criteria
- **Acceptable**: ≤10 minutes (baseline 8m + 25% buffer)
- **Unacceptable**: >20 minutes (auto-fail, 2.5x baseline)
- **Failure**: Build crashes or container exits with error

## Optimization Groupings (Based on Research)

### Group A: Critical Individual Tests
- **Test 16**: `QEMU_CPU=max,pauth=off` - Disables expensive QARMA5 crypto (17x speedup)
- **Test 15**: `CARGO_BUILD_JOBS=1` - Prevents parallel linking SIGSEGV in QEMU emulation

### Group B: APT Redundancies (Test as Groups)
- **Tests 5-7**: apt-check methods (physical removal, disable, stub) - likely only need one
- **Tests 9-11**: APT/DPkg hooks (Post-Invoke, Success, DPkg) - may overlap functionality
- **Tests 1-4**: Debian environment vars (FRONTEND, PRIORITY, LISTCHANGES, MOTD) - test as set

### Group C: Rust Compilation Optimizations
- **Tests 17-21**: Rust flags (codegen-units, opt-level, lto, debuginfo, strip) - test combined impact

### Group D: Package Management
- **Tests 12-14**: dpkg options (force-confold, force-confdef, no-install-recommends)

## Expected Relationships

### Redundant Optimizations (Research-Based)
1. **apt-check methods (#5-7)**: Physical removal likely makes disable/stub unnecessary
2. **APT hooks (#9-11)**: DPkg::Post-Invoke may make APT hooks redundant
3. **Parallelism (#15,17)**: CARGO_BUILD_JOBS=1 and codegen-units=1 both limit parallelism
4. **Build speed (#18-21)**: Multiple Rust flags with similar goals (faster compilation)

### Independent Optimizations
1. **QEMU_CPU (#16)**: Addresses CPU emulation bottleneck, independent of other optimizations
2. **DEBIAN_FRONTEND (#1)**: Prevents interactive prompts, unrelated to performance
3. **Detection (#22)**: Conditional application logic, not a performance optimization

## Binary Reduction Testing Strategy

### Phase 1: Critical Tests (Individual)
1. Test baseline (all enabled) = ~8m
2. Test without QEMU_CPU (expect >20m failure)
3. Test without CARGO_BUILD_JOBS (expect crash)

### Phase 2: Group Testing
1. Remove entire Group B (APT optimizations) - measure impact
2. Remove entire Group C (Rust flags) - measure impact
3. Remove entire Group D (package management) - measure impact

### Phase 3: Minimal Set Identification
1. Start with critical + highest impact groups
2. Add back optimizations until <10m achieved
3. Document final minimal set

## Notes
- Use `docker exec $(docker ps -qf name=act | head -1) pmap 1 | grep qemu` to verify QEMU detection
- Baseline was established after implementing all optimizations (10x improvement from original)
- Some optimizations may be redundant and can be safely removed if no impact
- >20m build time is unacceptable regardless of success - indicates insufficient optimization