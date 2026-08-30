FROM mcr.microsoft.com/playwright:v1.62.0-noble

ARG CHROME_VERSION=152.0.7977.54
ARG CHROME_SHA256=88af83664e1e5f79dc1c1378d0699b98dddd69690a748addf4ccbe322bfacedf

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl unzip \
    && curl -fsSL "https://storage.googleapis.com/chrome-for-testing-public/${CHROME_VERSION}/linux64/chrome-linux64.zip" -o /tmp/chrome-linux64.zip \
    && echo "${CHROME_SHA256}  /tmp/chrome-linux64.zip" | sha256sum -c - \
    && unzip -q /tmp/chrome-linux64.zip -d /opt/chrome \
    && test -x /opt/chrome/chrome-linux64/chrome \
    && rm -f /tmp/chrome-linux64.zip \
    && rm -rf /var/lib/apt/lists/*

ENV EASYNET_REMOTEAPP_BROWSER_CHROME_PATH=/opt/chrome/chrome-linux64/chrome
