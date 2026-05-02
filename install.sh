#!/bin/sh
#
# apkg installer — downloads the latest release from GitHub and installs it
# to ~/.apkg/bin, updating your shell RC file to put it on PATH.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/apkg-ai/cli/main/install.sh | sh
#
# Options (via environment or flags):
#   APKG_VERSION=v0.1.0       Pin a specific version (default: latest release).
#   --version <X.Y.Z>         Same as APKG_VERSION.
#   --install-dir <path>      Install to a custom directory (default: ~/.apkg/bin).
#                             When set, the shell RC file is NOT modified.
#   --no-modify-path          Install but do not touch any shell RC file.

set -eu

REPO="apkg-ai/cli"
DEFAULT_BIN_DIR="$HOME/.apkg/bin"

# Keep these byte-identical with src/commands/add_to_path.rs so `apkg add-to-path`
# detects this block via MARKER and skips re-adding.
PATH_COMMENT="# Added by apkg"
PATH_EXPORT='export PATH="$HOME/.apkg/bin:$PATH"'
FISH_PATH_LINE='fish_add_path $HOME/.apkg/bin'
MARKER=".apkg/bin"

# ---------- helpers ----------

err() {
    printf '\033[31merror:\033[0m %s\n' "$1" >&2
    exit 1
}

info() {
    printf '\033[36m%s\033[0m\n' "$1" >&2
}

success() {
    printf '\033[32m%s\033[0m\n' "$1" >&2
}

warn() {
    printf '\033[33mwarning:\033[0m %s\n' "$1" >&2
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"
}

download() {
    # download URL DEST
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        err "neither curl nor wget is installed"
    fi
}

# ---------- arg parsing ----------

version="${APKG_VERSION:-}"
install_dir=""
modify_path=1

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            shift
            [ $# -gt 0 ] || err "--version requires an argument"
            version="$1"
            ;;
        --install-dir)
            shift
            [ $# -gt 0 ] || err "--install-dir requires an argument"
            install_dir="$1"
            modify_path=0
            ;;
        --no-modify-path)
            modify_path=0
            ;;
        -h|--help)
            sed -n '3,15p' "$0" 2>/dev/null || true
            exit 0
            ;;
        *)
            err "unknown argument: $1"
            ;;
    esac
    shift
done

[ -n "$install_dir" ] || install_dir="$DEFAULT_BIN_DIR"

# ---------- platform detection ----------

uname_s="$(uname -s)"
case "$uname_s" in
    Linux)  os="linux" ;;
    Darwin) os="darwin" ;;
    *)
        err "unsupported OS: $uname_s. See https://github.com/$REPO/releases to download manually."
        ;;
esac

uname_m="$(uname -m)"
case "$uname_m" in
    x86_64|amd64) arch="amd64" ;;
    arm64|aarch64) arch="arm64" ;;
    *)
        err "unsupported architecture: $uname_m. See https://github.com/$REPO/releases to download manually."
        ;;
esac

# ---------- resolve version ----------

if [ -z "$version" ]; then
    info "Resolving latest version..."
    tmp_tag="$(mktemp)"
    trap 'rm -f "$tmp_tag"' EXIT
    download "https://api.github.com/repos/$REPO/releases/latest" "$tmp_tag"
    # Grep the first tag_name line; accept any non-escaped string between quotes.
    version="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp_tag" | head -n 1)"
    rm -f "$tmp_tag"
    trap - EXIT
    [ -n "$version" ] || err "failed to resolve latest version from GitHub API"
fi

# Tag must start with 'v'; accept both forms and normalize.
case "$version" in
    v*) tag="$version" ;;
    *)  tag="v$version" ;;
esac

info "Installing apkg $tag for $os/$arch..."

# ---------- download + verify ----------

asset="apkg-$os-$arch.tar.gz"
asset_url="https://github.com/$REPO/releases/download/$tag/$asset"
sha_url="$asset_url.sha256"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

download "$asset_url" "$work/$asset" || err "failed to download $asset_url"

# Checksum verification (best-effort: warn and skip if no tool available).
if command -v sha256sum >/dev/null 2>&1; then
    sha_tool="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
    sha_tool="shasum -a 256"
else
    sha_tool=""
fi

if [ -n "$sha_tool" ]; then
    download "$sha_url" "$work/$asset.sha256" || err "failed to download checksum from $sha_url"
    expected="$(awk '{print $1}' "$work/$asset.sha256")"
    actual="$(cd "$work" && $sha_tool "$asset" | awk '{print $1}')"
    if [ "$expected" != "$actual" ]; then
        err "checksum mismatch for $asset (expected $expected, got $actual)"
    fi
    info "Checksum verified."
else
    warn "no sha256sum/shasum found; skipping checksum verification"
fi

# ---------- extract + install ----------

tar -xzf "$work/$asset" -C "$work"
[ -f "$work/apkg" ] || err "tarball did not contain expected 'apkg' binary"

mkdir -p "$install_dir"
# Remove any existing file or symlink so mv doesn't fail.
rm -f "$install_dir/apkg"
mv "$work/apkg" "$install_dir/apkg"
chmod 755 "$install_dir/apkg"

success "Installed apkg to $install_dir/apkg"

# ---------- update shell RC ----------

update_rc() {
    # update_rc RC_PATH LINE
    rc_path="$1"
    line="$2"

    if [ -f "$rc_path" ] && grep -qF "$MARKER" "$rc_path"; then
        info "PATH entry already present in $rc_path"
        return 0
    fi

    rc_dir="$(dirname "$rc_path")"
    [ -d "$rc_dir" ] || mkdir -p "$rc_dir"

    {
        printf '\n'
        printf '%s\n' "$PATH_COMMENT"
        printf '%s\n' "$line"
    } >> "$rc_path"

    success "Added PATH entry to $rc_path"
}

if [ "$modify_path" -eq 1 ]; then
    shell_name="${SHELL##*/}"
    case "$shell_name" in
        zsh)
            update_rc "$HOME/.zshrc" "$PATH_EXPORT"
            rc_hint="$HOME/.zshrc"
            ;;
        fish)
            update_rc "$HOME/.config/fish/config.fish" "$FISH_PATH_LINE"
            rc_hint="$HOME/.config/fish/config.fish"
            ;;
        bash|sh|"")
            # Match add_to_path.rs: prefer .bashrc, fall back to .bash_profile, then .profile.
            if [ -f "$HOME/.bashrc" ]; then
                rc_hint="$HOME/.bashrc"
            elif [ -f "$HOME/.bash_profile" ]; then
                rc_hint="$HOME/.bash_profile"
            elif [ -f "$HOME/.profile" ]; then
                rc_hint="$HOME/.profile"
            else
                rc_hint="$HOME/.bashrc"
            fi
            update_rc "$rc_hint" "$PATH_EXPORT"
            ;;
        *)
            warn "unknown shell '$shell_name'; skipping RC file update"
            rc_hint=""
            ;;
    esac

    printf '\n' >&2
    printf 'To start using apkg, either:\n' >&2
    printf '\n' >&2
    printf '  1. Open a new terminal window, or\n' >&2
    if [ -n "$rc_hint" ]; then
        printf '  2. Run: source %s\n' "$rc_hint" >&2
    fi
    printf '\n' >&2
else
    printf '\n' >&2
    printf 'Add this directory to your PATH:\n' >&2
    printf '  %s\n' "$install_dir" >&2
    printf '\n' >&2
fi

# Print the installed version as a smoke check.
if "$install_dir/apkg" --version >/dev/null 2>&1; then
    installed_version="$("$install_dir/apkg" --version)"
    success "Installed: $installed_version"
fi
