ARG BASE_IMAGE=easynet/device-e2e:local
FROM ${BASE_IMAGE}

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        openbox \
        socat \
        python3-tk \
        x11-apps \
        x11-utils \
        xdotool \
        xvfb \
    && rm -rf /var/lib/apt/lists/*

COPY tools/fixtures/remoteapp-linux-x11-sentinel.py /opt/easynet/e2e/remoteapp-linux-x11-sentinel.py

ENTRYPOINT []
CMD ["/bin/sh", "-lc", "trap : TERM INT; sleep infinity & wait"]
