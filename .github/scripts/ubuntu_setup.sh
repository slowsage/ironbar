#!/usr/bin/env bash
set -e

# TOP 6 APT OPTIMIZATIONS TEST
# Global: QEMU_CPU + CARGO_BUILD_JOBS + RUSTFLAGS (from binary.yml)

# #6: Physical apt-check removal (additional protection)
sudo mv /usr/lib/update-notifier/apt-check /usr/lib/update-notifier/apt-check.disabled 2>/dev/null || true
echo '#!/bin/bash
exit 0' | sudo tee /usr/lib/update-notifier/apt-check > /dev/null
sudo chmod +x /usr/lib/update-notifier/apt-check

# #2: Remove command-not-found (prevents 91.5% CPU from cnf-update-db)
sudo rm -f /etc/apt/apt.conf.d/50command-not-found

# #3: Disable update-motd (prevents 100% CPU from update-notifier)
sudo chmod -x /usr/lib/update-notifier/update-motd-updates-available 2>/dev/null || true
sudo rm -f /etc/update-motd.d/90-updates-available 2>/dev/null || true

sudo dpkg --configure -a || true

# #5: APT::Update::Post-Invoke="" for apt-get update
sudo apt-get update \
  -o APT::Update::Post-Invoke::="" \
  -o APT::Update::Post-Invoke-Success::=""

# #1: DPkg::Post-Invoke="" (prevents 95.6% CPU from deb-systemd-helper)
# #4: APT::Update::Post-Invoke-Success="" for apt-get install
sudo apt-get install -y --no-install-recommends \
  -o DPkg::Post-Invoke::="" \
  -o APT::Update::Post-Invoke-Success::="" \
  libssl-dev libgtk-3-dev libgtk-layer-shell-dev \
  libinput-dev libdbusmenu-gtk3-dev libdbus-1-dev \
  libpulse-dev libluajit-5.1-dev