#!/bin/sh
set -eu

REPO="LuMiSxh/Acus"
INSTALL_DIR="${ACUS_INSTALL_DIR:-$HOME/.local/bin}"
INSTALL_SCOPE="${ACUS_INSTALL_SCOPE:-user}"

main() {
    os="$(detect_os)"
    arch="$(detect_arch)"
    target="${arch}-${os}"

    latest="$(fetch_latest_tag)"
    if [ -z "$latest" ]; then
        err "could not determine latest release"
    fi

    url="https://github.com/${REPO}/releases/download/${latest}/acus-${target}.tar.gz"
    say "downloading acus ${latest} for ${target}..."

    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$tmpdir/acus.tar.gz"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$tmpdir/acus.tar.gz" "$url"
    else
        err "need curl or wget"
    fi

    tar xzf "$tmpdir/acus.tar.gz" -C "$tmpdir"
    mkdir -p "$INSTALL_DIR"
    mv "$tmpdir/acus" "$INSTALL_DIR/acus"
    chmod +x "$INSTALL_DIR/acus"
    say "installed acus to ${INSTALL_DIR}/acus"

    # Installs the agent skill for Claude Code and Codex; ACUS_NO_SKILL=1 skips it.
    if [ -z "${ACUS_NO_SKILL:-}" ]; then
        "$INSTALL_DIR/acus" skill --install
    fi

    add_to_path
}

# Adds $INSTALL_DIR to PATH, either for the current user (default, appends to
# the shell rc file) or system-wide (writes /etc/profile.d/acus.sh, needs
# root). Choose with ACUS_INSTALL_SCOPE=user|system.
add_to_path() {
    if echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        return
    fi

    line="export PATH=\"${INSTALL_DIR}:\$PATH\""

    case "$INSTALL_SCOPE" in
        system)
            profile_file="/etc/profile.d/acus.sh"
            if [ -f "$profile_file" ] && grep -qF "$INSTALL_DIR" "$profile_file"; then
                return
            fi
            if [ -w /etc/profile.d ] || [ "$(id -u)" = "0" ]; then
                printf '%s\n' "$line" > "$profile_file"
                say "added ${INSTALL_DIR} to system-wide PATH via ${profile_file}"
            elif command -v sudo >/dev/null 2>&1; then
                printf '%s\n' "$line" | sudo tee "$profile_file" >/dev/null
                say "added ${INSTALL_DIR} to system-wide PATH via ${profile_file} (sudo)"
            else
                say ""
                say "could not write ${profile_file} (need root) — add manually:"
                say "  $line"
            fi
            ;;
        *)
            rc_file="$HOME/.profile"
            case "$(basename "${SHELL:-}")" in
                zsh)  rc_file="$HOME/.zshrc" ;;
                bash) rc_file="$HOME/.bashrc" ;;
            esac
            if [ -f "$rc_file" ] && grep -qF "$INSTALL_DIR" "$rc_file"; then
                return
            fi
            printf '\n# added by acus installer\n%s\n' "$line" >> "$rc_file"
            say "added ${INSTALL_DIR} to PATH in ${rc_file} (restart your shell or run: source ${rc_file})"
            ;;
    esac
}

detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "unknown-linux-gnu" ;;
        Darwin*) echo "apple-darwin" ;;
        *)       err "unsupported OS — use install.ps1 on Windows" ;;
    esac
}

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  echo "x86_64" ;;
        arm64|aarch64) echo "aarch64" ;;
        *)             err "unsupported architecture: $(uname -m)" ;;
    esac
}

fetch_latest_tag() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/'
    else
        wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/'
    fi
}

say() { printf '%s\n' "$1"; }
err() { say "error: $1" >&2; exit 1; }

main
