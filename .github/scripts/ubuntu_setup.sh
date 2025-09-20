#!/usr/bin/env bash
set -e

# GitHub's hosted runners run this as a non-root user, so the apt steps need
# sudo. Under `act` the container is already root, and on the emulated arm64
# image setuid `sudo` is broken (QEMU user-mode ignores the setuid bit, giving
# "sudo: effective uid is not 0 ... nosuid"). Shim sudo away when already root.
if [ "$(id -u)" -eq 0 ]; then
  sudo() { "$@"; }
fi

if pmap 1 2>/dev/null | grep -q qemu; then
  {
    echo "QEMU_CPU=max,pauth=off" # need to export as well
    echo "CARGO_BUILD_JOBS=1"
    echo "RUSTFLAGS=-C codegen-units=1 -C opt-level=0 -C lto=false -C debuginfo=0 -C strip=symbols"
  } >>"$GITHUB_ENV"

  sudo mv /usr/lib/update-notifier/apt-check{,.disabled} 2>/dev/null || :
  sudo ln -sf /bin/true /usr/lib/update-notifier/apt-check
  sudo rm -f /etc/apt/apt.conf.d/50command-not-found /etc/update-motd.d/90-updates-available
  sudo chmod -x /usr/lib/update-notifier/update-motd-updates-available 2>/dev/null || :
fi

sudo apt-get update -o APT::Update::Post-Invoke::="" -o APT::Update::Post-Invoke-Success::=""
sudo apt-get install -y --no-install-recommends -o DPkg::Post-Invoke::="" -o APT::Update::Post-Invoke-Success::="" "$@"
