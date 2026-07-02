# EasyNet Hub-in-Docker — runtime-only image (binaries built outside)
# ===================================================================
#
# Binaries are produced by cross-compiling in silan's existing
# `easynet-linux-build:bookworm-arm64` image:
#
#   docker run --rm --platform linux/arm64 \
#       -v $HOME/Documents/Github:/work \
#       -v easynet-cargo-cache-arm64:/usr/local/cargo/registry \
#       -w /work/EasyNet-Cli \
#       easynet-linux-build:bookworm-arm64 \
#       cargo build --release --target aarch64-unknown-linux-gnu \
#       --bin easynet --bin easynet-daemon --features axon-pb
#
# This Dockerfile then just packs the resulting binaries into a
# minimal runtime image. No compilation here = ~10 second build.
#
# Build context = EasyNet-Cli (sees binaries at
# `target/aarch64-unknown-linux-gnu/release/{easynet,easynet-daemon}`).

FROM debian:12-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        jq \
        procps \
        libssl3 \
        # Runtime libs the daemon links against because of the xcap
        # screen-capture dep (RFC-005 v3.2 A4/A8). Same list as
        # silan's existing easynet/device-e2e:local image
        # (engineering/docker/e2e/Dockerfile.device).
        libasound2 \
        libegl1 \
        libgbm1 \
        libpipewire-0.3-0 \
        libwayland-client0 \
        libwayland-server0 \
        libxcb1 \
    && rm -rf /var/lib/apt/lists/*

COPY target/aarch64-unknown-linux-gnu/release/easynet         /usr/local/bin/easynet
COPY target/aarch64-unknown-linux-gnu/release/easynet-daemon  /usr/local/bin/easynet-daemon
RUN chmod +x /usr/local/bin/easynet /usr/local/bin/easynet-daemon

ENV EASYNET_PAGES_PORT=8787
ENV EASYNET_PAGES_USER=alice
ENV EASYNET_PAGES_REALM=easynet.run
ENV HOME=/root

RUN mkdir -p /sites /root/.easynet

COPY engineering/docker/hub-pages/full/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

EXPOSE 8787

CMD ["/usr/local/bin/entrypoint.sh"]
