#!/bin/sh
# EasyNet CLI installer
#
# Usage:
#   curl -sSf https://easynet.run/install | sudo sh   # recommended
#   sudo curl -sSf https://easynet.run/install | sh   # also works (we
#                                                       detect this)
#
# Why two patterns
# ----------------
# /usr/local/bin requires root to write on most systems. The script
# needs an effective uid of 0 for the binary install step. Two ways
# to get there:
#
#   * pipe-into-sudo:    `curl … | sudo sh`. sudo proxies the SHELL,
#     so the entire script runs as root, no in-script sudo prompts.
#     This is the cleanest, what the README recommends.
#
#   * sudo-into-curl:    `sudo curl … | sh`. People type this from
#     muscle memory. Only the curl is root; the piped `sh` is the
#     invoking user. The script detects this and re-execs itself
#     under sudo so the user doesn't have to know the difference.
#
# Anti-pattern caught here: the prior version unconditionally fell
# through to per-command `sudo mv`. Inside `curl | sh`, stdin is the
# tarball stream, not a tty — sudo's password prompt has nowhere to
# render and the script stalls silently after `echo "Need sudo …"`.
# Detect-then-reexec lifts the credential check out of the per-mv
# loop and into a single "are we root?" gate.
#
# Author: Silan.Hu <silan.hu@u.nus.edu>
set -eu

BASE_URL="https://easynet.run/download"
INSTALL_DIR="/usr/local/bin"
INCLUDE_DIR="/usr/local/include/easynet"
DOC_DIR="/usr/local/share/doc/easynet"

# Resolve the home directory we should plant ~/.easynet under. When the
# script runs under `sudo`, $HOME is /root — which is wrong: the user
# who ran `sudo` is the one who'll be invoking `easynet` and dlopening
# the dendrite bridge from $HOME/.easynet/dendrite-bridge/. SUDO_USER
# is the hint that lets us recover the real home; fall back to $HOME
# if running as actual root (no SUDO_USER set).
if [ -n "${SUDO_USER:-}" ] && [ "${SUDO_USER}" != "root" ]; then
    REAL_USER="$SUDO_USER"
    REAL_HOME=$(getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6)
    [ -z "$REAL_HOME" ] && REAL_HOME=$(eval echo "~$SUDO_USER")
else
    REAL_USER="$(id -un)"
    REAL_HOME="$HOME"
fi
EASYNET_HOME="$REAL_HOME/.easynet"
NATIVE_DIR="$EASYNET_HOME/dendrite-bridge/native"

main() {
    detect_platform
    ensure_root
    download_and_install
    setup_env
    cleanup_stale_binaries
    reload_shell
    echo ""
    echo "  ✓ EasyNet CLI installed successfully!"
    echo ""
    echo "    easynet                      → $INSTALL_DIR/"
    echo "    easynet-daemon               → $INSTALL_DIR/"
    echo "    easynet-keyring              → $INSTALL_DIR/"
    echo "    libaxon_dendrite_bridge.$LIB_EXT  → $NATIVE_DIR/"
    echo "    easynet_cli.h                → $INCLUDE_DIR/"
    echo "    easynet_cli.exports.v5       → $INCLUDE_DIR/"
    echo "    ffi-abi-v5.md                → $DOC_DIR/"
    echo ""
    if [ -n "${PROFILE:-}" ]; then
        echo "  To activate in this terminal, run:"
        echo ""
        echo "    source $PROFILE"
        echo ""
    fi
    echo "  Then run 'easynet --help' to get started."
    echo ""
}

# ensure_root either confirms we're already running as root, or
# re-execs the script under sudo so the rest of main() can mv into
# /usr/local/bin without per-step credential prompts. The re-exec
# preserves the inbound stdin so a piped tarball or env file (none
# today, but future-proofing) survives the hop.
ensure_root() {
    if [ "$(id -u)" -eq 0 ]; then
        return 0
    fi

    # Two situations to distinguish:
    #
    #   a. user has a tty + sudo is available  → re-exec under sudo
    #      so the password prompt has somewhere to render, and the
    #      ENTIRE script runs as root from this point.
    #   b. running headless / piped / no sudo  → bail with a clear
    #      message rather than fall through to a hanging `sudo mv`.
    #
    # The re-exec uses `exec sudo -E sh -c "$SCRIPT_BODY"` rather
    # than `sudo $0` because $0 inside `curl | sh` is `sh` itself
    # with no script file on disk to re-invoke. We capture the
    # script body via /proc/self/fd/0 fall-through is hard from
    # inside the same pipe; instead we instruct the user.
    if ! command -v sudo >/dev/null 2>&1; then
        echo ""
        echo "  ! ${INSTALL_DIR} is not writable and 'sudo' is not"
        echo "    installed. Either run this installer as root, or"
        echo "    install sudo first."
        exit 1
    fi

    # We're inside `curl … | sh`, so /proc/self/fd/255 (the script
    # file) is a pipe we can no longer rewind. The clean fix is to
    # have the user re-pipe through sudo; the friendly fix is to
    # write the body to a temp file and exec that. Pick the friendly
    # one — operators expect the installer to "just work" without
    # caring whether it ran sudo internally or via the pipe.
    SCRIPT_TMP=$(mktemp -t easynet-install.XXXXXX) || {
        echo "  ! could not create temp file for re-exec; please rerun as:" >&2
        echo "      curl -sSf https://easynet.run/install | sudo sh" >&2
        exit 1
    }
    # /dev/stdin is the pipe; we already drained it past `set -eu`,
    # but the body of the rest of the script lives in our argv when
    # the parent shell sourced us. The portable shape: download the
    # script ourselves to the temp file and exec it under sudo.
    if ! curl -sSfL "https://easynet.run/install" -o "$SCRIPT_TMP" 2>/dev/null; then
        rm -f "$SCRIPT_TMP"
        echo "  ! could not refetch installer for sudo re-exec." >&2
        echo "    Please run:" >&2
        echo "      curl -sSf https://easynet.run/install | sudo sh" >&2
        exit 1
    fi
    chmod +x "$SCRIPT_TMP"
    # Pass through SUDO_USER so the re-exec keeps the right home dir.
    echo "  Re-running under sudo for system install (you may be prompted for your password)..."
    exec sudo -E "$SCRIPT_TMP"
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

    # We're already root by the time we get here (ensure_root saw to
    # that). Plain mv into INSTALL_DIR — no per-step sudo, no tty
    # juggling.
    #
    # The transport-plane rollout ships three binaries:
    # `easynet` (user-facing CLI), `easynet-daemon` (long-running
    # control + InvocationServer sidecar), and `easynet-keyring`
    # (device-signing vault sidecar). Treat them as required
    # artefacts: if the tarball is missing one, the release is
    # malformed and the installer should fail loudly.
    mv "${TMPDIR}/easynet"        "${INSTALL_DIR}/easynet"
    mv "${TMPDIR}/easynet-daemon" "${INSTALL_DIR}/easynet-daemon"
    mv "${TMPDIR}/easynet-keyring" "${INSTALL_DIR}/easynet-keyring"
    chmod +x "${INSTALL_DIR}/easynet" "${INSTALL_DIR}/easynet-daemon" "${INSTALL_DIR}/easynet-keyring"

    # Install the generic C ABI v5 contract alongside the runtime artefacts.
    # Language bindings compile against the header; release/CI tooling uses
    # the exact export allowlist; the spec carries ownership rules.
    mkdir -p "$INCLUDE_DIR" "$DOC_DIR"
    mv "${TMPDIR}/include/easynet_cli.h" "${INCLUDE_DIR}/easynet_cli.h"
    mv "${TMPDIR}/include/easynet_cli.exports.v5" "${INCLUDE_DIR}/easynet_cli.exports.v5"
    mv "${TMPDIR}/docs/spec/ffi-abi-v5.md" "${DOC_DIR}/ffi-abi-v5.md"

    # Install dendrite bridge library under the REAL user's home so
    # the daemon can dlopen it without LD_LIBRARY_PATH gymnastics.
    # We're root right now, so set ownership back to the real user
    # afterwards or the daemon (running as the user) hits EACCES on
    # the parent dir.
    mkdir -p "$NATIVE_DIR"
    mv "${TMPDIR}/libaxon_dendrite_bridge.${LIB_EXT}" "$NATIVE_DIR/"
    if [ -n "${SUDO_USER:-}" ] && [ "${SUDO_USER}" != "root" ]; then
        chown -R "$SUDO_USER" "$EASYNET_HOME" 2>/dev/null || true
    fi
}

setup_env() {
    ENV_LINE="export EASYNET_DENDRITE_BRIDGE_LIB=\"$NATIVE_DIR/libaxon_dendrite_bridge.${LIB_EXT}\""

    # Detect shell profile. Even though we're root, we want to write
    # into the REAL user's profile, not /root/.zshrc — the real user
    # is the one whose interactive shells need the env var.
    PROFILE=""
    REAL_SHELL="${SHELL:-/bin/sh}"
    if [ -n "${SUDO_USER:-}" ] && [ "${SUDO_USER}" != "root" ]; then
        REAL_SHELL=$(getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f7)
        [ -z "$REAL_SHELL" ] && REAL_SHELL="/bin/sh"
    fi
    case "$(basename "$REAL_SHELL")" in
        zsh)
            PROFILE="$REAL_HOME/.zshrc" ;;
        bash)
            if [ -f "$REAL_HOME/.bash_profile" ]; then
                PROFILE="$REAL_HOME/.bash_profile"
            else
                PROFILE="$REAL_HOME/.bashrc"
            fi ;;
        *)
            [ -f "$REAL_HOME/.profile" ] && PROFILE="$REAL_HOME/.profile" ;;
    esac

    # Set for current session (so cleanup_stale_binaries / reload
    # below can use it).
    export EASYNET_DENDRITE_BRIDGE_LIB="$NATIVE_DIR/libaxon_dendrite_bridge.${LIB_EXT}"

    # Persist to shell profile. We're root, so chown the file back
    # to the real user after appending so they can later edit it.
    if [ -n "$PROFILE" ]; then
        if ! grep -q "EASYNET_DENDRITE_BRIDGE_LIB" "$PROFILE" 2>/dev/null; then
            {
                echo ""
                echo "# EasyNet dendrite bridge"
                echo "$ENV_LINE"
            } >> "$PROFILE"
            if [ -n "${SUDO_USER:-}" ] && [ "${SUDO_USER}" != "root" ]; then
                chown "$SUDO_USER" "$PROFILE" 2>/dev/null || true
            fi
            echo "  Added EASYNET_DENDRITE_BRIDGE_LIB to $PROFILE"
        fi
    else
        echo "  Could not detect shell profile. Add this to your shell config:"
        echo "    $ENV_LINE"
    fi
}

reload_shell() {
    # Source the profile to make env vars available in the current
    # session. This is a best-effort — if running via pipe (curl |
    # sh), the sourced vars won't persist in the parent shell, but
    # they'll be available for any easynet commands run within this
    # script.
    if [ -n "${PROFILE:-}" ] && [ -f "$PROFILE" ]; then
        # shellcheck disable=SC1090
        . "$PROFILE" 2>/dev/null || true
    fi
}

cleanup_stale_binaries() {
    # Remove stale easynet/easynet-daemon/easynet-keyring/axon-runtime binaries from
    # other PATH dirs
    # that would shadow the freshly installed copy. We're root here,
    # so direct rm — no nested sudo dance.
    for bin in easynet easynet-daemon easynet-keyring axon-runtime; do
        IFS=:
        for dir in $PATH; do
            unset IFS
            [ "$dir" = "$INSTALL_DIR" ] && continue
            candidate="$dir/$bin"
            [ -x "$candidate" ] || continue
            echo "  Removing stale $candidate (shadows ${INSTALL_DIR}/${bin})"
            rm -f "$candidate" 2>/dev/null || \
                echo "  Warning: could not remove $candidate — please delete it manually"
        done
        unset IFS
    done
}

main
