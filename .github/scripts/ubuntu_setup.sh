#!/usr/bin/env bash
set -e

# APT fix for ARM64 cross-compilation (Ubuntu 24.04 mirror+file issue)
if [ -n "$CROSS_DEB_ARCH" ]; then
  sudo cp /etc/apt/sources.list.d/ubuntu.sources /etc/apt/sources.list.d/ubuntu.sources.bak
  printf '# Ubuntu sources with architecture-specific mirrors\n\n## AMD64 repositories\nTypes: deb\nURIs: http://azure.archive.ubuntu.com/ubuntu/ https://archive.ubuntu.com/ubuntu/\nSuites: noble noble-updates noble-backports\nComponents: main universe restricted multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\nArchitectures: amd64\n\n## AMD64 security updates\nTypes: deb\nURIs: https://security.ubuntu.com/ubuntu/\nSuites: noble-security\nComponents: main universe restricted multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\nArchitectures: amd64\n\n## ARM64 repositories\nTypes: deb\nURIs: http://ports.ubuntu.com/ubuntu-ports/\nSuites: noble noble-updates noble-backports\nComponents: main universe restricted multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\nArchitectures: arm64\n\n## ARM64 security updates\nTypes: deb\nURIs: http://ports.ubuntu.com/ubuntu-ports/\nSuites: noble-security\nComponents: main universe restricted multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\nArchitectures: arm64\n' | sudo tee /etc/apt/sources.list.d/ubuntu.sources > /dev/null
  
  sudo dpkg --add-architecture "$CROSS_DEB_ARCH"
  sudo apt-get update
  sudo apt-get install -y gcc-aarch64-linux-gnu g++-aarch64-linux-gnu pkg-config
  rustup target add aarch64-unknown-linux-gnu
fi

# Install libraries for the target architecture
# For native x86_64: no suffix needed
# For cross-compile: need :arm64 suffix
ARCH_SUFFIX="${CROSS_DEB_ARCH:+:$CROSS_DEB_ARCH}"

sudo apt-get update && sudo apt-get install -y \
  libssl-dev${ARCH_SUFFIX} \
  libgtk-3-dev${ARCH_SUFFIX} \
  libgtk-layer-shell-dev${ARCH_SUFFIX} \
  libinput-dev${ARCH_SUFFIX} \
  libdbusmenu-gtk3-dev${ARCH_SUFFIX} \
  libpulse-dev${ARCH_SUFFIX} \
  libluajit-5.1-dev${ARCH_SUFFIX} \
  libdbus-1-dev${ARCH_SUFFIX} \
  libgdk-pixbuf-2.0-dev${ARCH_SUFFIX} \
  libcairo2-dev${ARCH_SUFFIX} \
  libatk1.0-dev${ARCH_SUFFIX} \
  libpango1.0-dev${ARCH_SUFFIX}