# Native Linux build environment for the EasyNet Runtime artifact bundle.
#
# `native-media` links platform capture/input dependencies whose build scripts
# use pkg-config. Building those crates in Linux keeps their system-library
# contract explicit and avoids an incomplete macOS-hosted cross sysroot.
FROM rust:1.95-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config clang lld cmake nasm protobuf-compiler \
        libwayland-dev libpipewire-0.3-dev \
        libxcb1-dev libxrandr-dev libxinerama-dev \
        libegl-dev libgles-dev \
        libgbm-dev libdrm-dev \
        libxkbcommon-dev libdbus-1-dev \
        libssl-dev \
        libasound2-dev libpulse-dev \
    && rm -rf /var/lib/apt/lists/*

ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
      "linux/amd64") rustup target add x86_64-unknown-linux-gnu ;; \
      "linux/arm64") rustup target add aarch64-unknown-linux-gnu ;; \
      *) echo "unsupported TARGETPLATFORM: $TARGETPLATFORM" >&2; exit 1 ;; \
    esac

WORKDIR /work
