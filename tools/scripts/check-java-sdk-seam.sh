#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
BUILD_DIR="$ROOT/target/java-sdk-seam"

if [[ ! -f "$ROOT/sdk/java/pom.xml" ]]; then
  echo "check-java-sdk-seam: missing Maven package metadata" >&2
  exit 1
fi

mvn -q -f "$ROOT/sdk/java/pom.xml" -DskipTests package

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/classes"

sources_file="$BUILD_DIR/sources.txt"
find "$ROOT/sdk/java/src/main/java" "$ROOT/sdk/java/src/test/java" -name '*.java' | sort >"$sources_file"
if [[ ! -s "$sources_file" ]]; then
  echo "check-java-sdk-seam: missing Java sources" >&2
  exit 1
fi

javac -Xlint:all -Werror -d "$BUILD_DIR/classes" @"$sources_file"
java -cp "$BUILD_DIR/classes" run.easynet.daemon.RuntimeCoreSeamTest

address_terms='U''RI|U''ri|u''ri'
if grep -R -nE "\\b($address_terms)\\b|axon\\.v1|protobuf|easynet\\.run/axon|axonP[Bb]|AxonP[Bb]" \
  "$ROOT/sdk/java/pom.xml" \
  "$ROOT/sdk/java/README.md" \
  "$ROOT/sdk/java/src" \
  >/tmp/easynet-java-sdk-seam-grep.$$ 2>/dev/null; then
  cat /tmp/easynet-java-sdk-seam-grep.$$ >&2
  rm -f /tmp/easynet-java-sdk-seam-grep.$$
  echo "check-java-sdk-seam: Java seam leaked forbidden naming or Axon/proto symbols" >&2
  exit 1
fi
rm -f /tmp/easynet-java-sdk-seam-grep.$$

echo "check-java-sdk-seam ok"
