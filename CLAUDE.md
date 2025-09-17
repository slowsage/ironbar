# Ironbar ARM64 Cross-Compilation with Act

## Overview
Complete ARM64 cross-compilation workflow for Ironbar using GitHub Actions locally with Act. This setup provides a reliable, repeatable build process that works on Ubuntu 24.04.

## Prerequisites

### Act Configuration (.actrc)
```
-P ubuntu-24.04=ghcr.io/christopherhx/runner-images:ubuntu24-runner-large-latest
--container-architecture linux/amd64
--use-gitignore=false
-v
```

### Required Tools
- Docker
- Act (GitHub Actions runner)
- Rust toolchain (in container)

## Workflow Files

### Binary Workflow (.github/workflows/binary.yml)
Complete 8-step ARM64 build process:

1. **Check initial container state** - Verify Ubuntu 24.04 setup
2. **Fix APT sources for ARM64** - Replace mirror+file with direct URLs
3. **Add ARM64 architecture** - Enable multi-arch package support
4. **Add Rust ARM64 target** - Install aarch64-unknown-linux-gnu
5. **Install cross-compilation tools** - GCC ARM64 toolchain
6. **Install ARM64 libraries** - GTK3, SSL, PulseAudio, etc.
7. **Set ARM64 build environment** - Cross-compilation variables
8. **Build ARM64 binary** - Compile and verify with `file` command

### Test Scripts

#### run.sh
```bash
#!/bin/bash
echo "=== Cleaning up old Act containers ==="
docker ps -a --format "{{.Names}}" | grep act | xargs -r docker stop
docker ps -a --format "{{.Names}}" | grep act | xargs -r docker rm
echo "✓ Old containers cleaned up"

echo ""
echo "=== Running binary workflow with 30m timeout ==="
./poll.sh -t 1800 -- act -W .github/workflows/binary.yml workflow_dispatch
```

#### poll.sh
Monitoring script with timeout support for long-running builds.

## Key Technical Solutions

### APT Sources Fix
Ubuntu 24.04 uses `mirror+file` which breaks ARM64 packages. Fixed with:
```bash
# Replace mirror+file with direct architecture-specific sources
Types: deb
URIs: http://azure.archive.ubuntu.com/ubuntu/ https://archive.ubuntu.com/ubuntu/
Architectures: amd64

Types: deb  
URIs: http://ports.ubuntu.com/ubuntu-ports/
Architectures: arm64
```

### Cross-Compilation Environment
Essential variables set in each build step:
```bash
export AARCH64_UNKNOWN_LINUX_GNU_PKG_CONFIG=aarch64-linux-gnu-pkg-config
export PKG_CONFIG_ALLOW_CROSS=1
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
export AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export PKG_CONFIG=aarch64-linux-gnu-pkg-config
export PKG_CONFIG_SYSROOT_DIR=/
export PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/lib/pkgconfig
```

### Build Command
```bash
cargo build --locked --release --target aarch64-unknown-linux-gnu
```

## Testing Strategy

### Local Development
```bash
# Quick test of complete workflow
./run.sh

# Individual step testing
act -W .github/workflows/binary.yml -j debug workflow_dispatch

# Monitor progress with shorter intervals
./poll.sh -t 600 -- act -W .github/workflows/binary.yml workflow_dispatch
```

### Verification Steps
1. **No 404 errors** during ARM64 package installation
2. **Binary creation** at `target/aarch64-unknown-linux-gnu/release/ironbar`
3. **Architecture verification** with `file` command showing ARM64
4. **Size and permissions** verification

### Common Issues & Solutions

#### Container Cleanup
```bash
docker ps -a --format "{{.Names}}" | grep act | xargs -r docker stop
docker ps -a --format "{{.Names}}" | grep act | xargs -r docker rm
```

#### Permission Issues
```bash
sudo chown -R $(id -u):$(id -g) target/
```

#### Build Monitoring
- Use 1-minute intervals for active compilation monitoring
- Set Claude timeouts longer than sleep durations (> 2m)
- Monitor specific stages: setup (fast), libraries (medium), compilation (slow)

## Performance Notes

### Timing Expectations
- Setup steps (1-3): ~2-3 minutes
- Tool installation (4-5): ~2-3 minutes  
- Library installation (6): ~5-8 minutes
- Build compilation (8): ~8-15 minutes
- **Total**: ~18-30 minutes

### Build Stages
1. **Dependency download**: Fast, parallel downloads
2. **Compilation**: Slower, sequential crate building
3. **Linking**: Fast, final binary creation

## Success Criteria
- ✅ All 8 workflow steps complete without errors
- ✅ No ARM64 package 404 errors  
- ✅ Binary verified as ARM64 ELF executable
- ✅ File size reasonable (~15-30MB for release build)
- ✅ Complete process under 30 minutes

## Integration
This workflow is designed to be:
- **Standalone** - No external setup scripts required
- **Reproducible** - Same result every run
- **Verifiable** - Each step has clear success/failure indicators
- **Extensible** - Easy to add additional ARM64 targets