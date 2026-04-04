#!/bin/sh
# EasyNet CLI installer
# Usage: curl -sSf https://easynet.run/install | sh
set -eu

BASE_URL="https://easynet.run/download"
INSTALL_DIR="/usr/local/bin"
EASYNET_HOME="$HOME/.easynet"
NATIVE_DIR="$EASYNET_HOME/dendrite-bridge/native"

main() {
    detect_platform
    download_and_install
    setup_env
    echo ""
    echo "  EasyNet CLI installed successfully!"
    echo "    - easynet                      → $INSTALL_DIR/"
    echo "    - axon-runtime                 → $INSTALL_DIR/"
    echo "    - libaxon_dendrite_bridge.$LIB_EXT  → $NATIVE_DIR/"
    echo ""
    echo "  Run 'easynet --help' to get started."
    echo ""
}

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            OS_TAG="unknown-linux-gnu"
            LIB_EXT="so"
            ;;
        Darwin)
            OS_TAG="apple-darwin"
            LIB_EXT="dylib"
            ;;
        MINGW*|MSYS*|CYGWIN*)
            echo "On Windows, run this instead:"
            echo "  irm https://easynet.run/install.ps1 | iex"
            exit 1
            ;;
        *) echo "Unsupported OS: $OS"; exit 1 ;;
    esac

    case "$ARCH" in
        x86_64|amd64) ARCH_TAG="x86_64" ;;
        aarch64|arm64) ARCH_TAG="aarch64" ;;
        *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac

    TARGET="${ARCH_TAG}-${OS_TAG}"
    echo "Detected platform: ${TARGET}"
}

download_and_install() {
    URL="${BASE_URL}/easynet-${TARGET}.tar.gz"
    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    echo "Downloading ${URL}..."
    curl -sSfL "$URL" -o "${TMPDIR}/easynet.tar.gz"
    tar xzf "${TMPDIR}/easynet.tar.gz" -C "${TMPDIR}"

    # Install binaries
    if [ -w "$INSTALL_DIR" ]; then
        mv "${TMPDIR}/easynet" "${INSTALL_DIR}/easynet"
        mv "${TMPDIR}/axon-runtime" "${INSTALL_DIR}/axon-runtime"
        chmod +x "${INSTALL_DIR}/easynet" "${INSTALL_DIR}/axon-runtime"
    else
        echo "Need sudo to install to ${INSTALL_DIR}"
        sudo mv "${TMPDIR}/easynet" "${INSTALL_DIR}/easynet"
        sudo mv "${TMPDIR}/axon-runtime" "${INSTALL_DIR}/axon-runtime"
        sudo chmod +x "${INSTALL_DIR}/easynet" "${INSTALL_DIR}/axon-runtime"
    fi

    # Install dendrite bridge library
    mkdir -p "$NATIVE_DIR"
    mv "${TMPDIR}/libaxon_dendrite_bridge.${LIB_EXT}" "$NATIVE_DIR/"
}

setup_env() {
    ENV_LINE="export EASYNET_DENDRITE_BRIDGE_LIB=\"$NATIVE_DIR/libaxon_dendrite_bridge.${LIB_EXT}\""

    # Detect shell profile
    PROFILE=""
    if [ -n "${ZSH_VERSION:-}" ] || [ "$(basename "${SHELL:-}")" = "zsh" ]; then
        PROFILE="$HOME/.zshrc"
    elif [ -n "${BASH_VERSION:-}" ] || [ "$(basename "${SHELL:-}")" = "bash" ]; then
        if [ -f "$HOME/.bash_profile" ]; then
            PROFILE="$HOME/.bash_profile"
        else
            PROFILE="$HOME/.bashrc"
        fi
    elif [ -f "$HOME/.profile" ]; then
        PROFILE="$HOME/.profile"
    fi

    # Set for current session
    export EASYNET_DENDRITE_BRIDGE_LIB="$NATIVE_DIR/libaxon_dendrite_bridge.${LIB_EXT}"

    # Persist to shell profile
    if [ -n "$PROFILE" ]; then
        if ! grep -q "EASYNET_DENDRITE_BRIDGE_LIB" "$PROFILE" 2>/dev/null; then
            echo "" >> "$PROFILE"
            echo "# EasyNet dendrite bridge" >> "$PROFILE"
            echo "$ENV_LINE" >> "$PROFILE"
            echo "  Added EASYNET_DENDRITE_BRIDGE_LIB to $PROFILE"
            echo "  Restart your terminal or run: source $PROFILE"
        fi
    else
        echo "  Could not detect shell profile. Add this to your shell config:"
        echo "    $ENV_LINE"
    fi
}

main
