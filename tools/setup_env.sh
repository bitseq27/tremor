#!/usr/bin/env bash
# Recreates the build toolchain used for this project on a bare Linux sandbox.
# (The sandbox keeps /home/user across sessions but not installed packages.)
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
if ! command -v rustc >/dev/null 2>&1; then
  python3 -c "import urllib.request; urllib.request.urlretrieve('https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init','/tmp/rustup-init')"
  chmod +x /tmp/rustup-init
  /tmp/rustup-init -y --profile minimal --default-toolchain stable
fi
export PATH="$HOME/.cargo/bin:$PATH"
rustup target add x86_64-pc-windows-gnu
apt-get update -qq
apt-get install -y -qq gcc mingw-w64 zip \
  xvfb libxkbcommon0 libxkbcommon-x11-0 libvulkan1 mesa-vulkan-drivers \
  libxcursor1 libxi6 libxrandr2 libxinerama1 libxext6 libxfixes3 libxrender1
echo "toolchain ready: $(rustc --version)"
echo "note: build with -j 1 on low-RAM machines (see build log.md M0)"
