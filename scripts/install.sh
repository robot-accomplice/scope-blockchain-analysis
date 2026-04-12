#!/usr/bin/env bash
# Scope Blockchain Analysis — Installer
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/robot-accomplice/scope-blockchain-analysis/main/scripts/install.sh | bash
#
# Options (via environment variables):
#   SCOPE_INSTALL_DIR  — Override install directory (default: ~/.local/bin)
#   SCOPE_VERSION      — Install a specific version (default: latest)
#   SCOPE_NO_MODIFY_PATH — Set to 1 to skip PATH modification

set -euo pipefail

REPO="robot-accomplice/scope-blockchain-analysis"
BINARY_NAME="scope"
DEFAULT_INSTALL_DIR="${HOME}/.local/bin"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

info()  { printf "\033[1;34m==>\033[0m %s\n" "$*"; }
ok()    { printf "\033[1;32m==>\033[0m %s\n" "$*"; }
warn()  { printf "\033[1;33mwarn:\033[0m %s\n" "$*" >&2; }
err()   { printf "\033[1;31merror:\033[0m %s\n" "$*" >&2; exit 1; }

need_cmd() {
    if ! command -v "$1" > /dev/null 2>&1; then
        err "Required command '$1' not found. Please install it and retry."
    fi
}

# ---------------------------------------------------------------------------
# Detect platform
# ---------------------------------------------------------------------------

detect_platform() {
    local os arch suffix

    os="$(uname -s)"
    arch="$(uname -m)"

    case "${os}" in
        Linux)  os="linux" ;;
        Darwin) os="macos" ;;
        *)      err "Unsupported operating system: ${os}" ;;
    esac

    case "${arch}" in
        x86_64|amd64)  arch="x64" ;;
        aarch64|arm64) arch="arm64" ;;
        *)             err "Unsupported architecture: ${arch}" ;;
    esac

    suffix="${os}-${arch}"
    echo "${suffix}"
}

# ---------------------------------------------------------------------------
# Resolve version
# ---------------------------------------------------------------------------

resolve_version() {
    if [ -n "${SCOPE_VERSION:-}" ]; then
        # User-specified version — ensure it has a 'v' prefix
        case "${SCOPE_VERSION}" in
            v*) echo "${SCOPE_VERSION}" ;;
            *)  echo "v${SCOPE_VERSION}" ;;
        esac
        return
    fi

    info "Fetching latest release..."
    local latest
    latest="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | head -1 \
        | sed -E 's/.*"tag_name":\s*"([^"]+)".*/\1/')"

    if [ -z "${latest}" ]; then
        err "Could not determine latest version. Set SCOPE_VERSION explicitly."
    fi

    echo "${latest}"
}

# ---------------------------------------------------------------------------
# Download and verify
# ---------------------------------------------------------------------------

download_and_verify() {
    local version="$1" suffix="$2" tmpdir="$3"
    local base_url="https://github.com/${REPO}/releases/download/${version}"
    local tarball="scope-${suffix}.tar.gz"
    local checksum_file="${tarball}.sha256"
    local tarball_url="${base_url}/${tarball}"
    local checksum_url="${base_url}/${checksum_file}"

    info "Downloading ${BINARY_NAME} ${version} (${suffix})..."
    curl -fSL --progress-bar -o "${tmpdir}/${tarball}" "${tarball_url}" \
        || err "Download failed. Check that ${version} has a ${suffix} build at:\n  ${tarball_url}"

    # Attempt to download the checksum file (may not exist for older releases)
    if curl -fsSL -o "${tmpdir}/${checksum_file}" "${checksum_url}" 2>/dev/null; then
        info "Verifying SHA-256 checksum..."
        local expected actual
        expected="$(awk '{print $1}' "${tmpdir}/${checksum_file}")"
        actual="$(shasum -a 256 "${tmpdir}/${tarball}" | awk '{print $1}')"

        if [ "${expected}" != "${actual}" ]; then
            err "Checksum mismatch!\n  Expected: ${expected}\n  Got:      ${actual}\n\nThe download may be corrupted or tampered with."
        fi
        ok "Checksum verified."
    else
        warn "No checksum file found for this release — skipping verification."
        warn "Consider upgrading to a newer release that includes checksums."
    fi

    # Extract
    info "Extracting..."
    tar xzf "${tmpdir}/${tarball}" -C "${tmpdir}"

    if [ ! -f "${tmpdir}/${BINARY_NAME}" ]; then
        err "Expected binary '${BINARY_NAME}' not found in archive."
    fi
}

# ---------------------------------------------------------------------------
# Install binary
# ---------------------------------------------------------------------------

install_binary() {
    local tmpdir="$1"
    local install_dir="${SCOPE_INSTALL_DIR:-${DEFAULT_INSTALL_DIR}}"

    # Create install directory if needed
    mkdir -p "${install_dir}"

    info "Installing to ${install_dir}/${BINARY_NAME}..."
    mv "${tmpdir}/${BINARY_NAME}" "${install_dir}/${BINARY_NAME}"
    chmod +x "${install_dir}/${BINARY_NAME}"

    echo "${install_dir}"
}

# ---------------------------------------------------------------------------
# Add to PATH
# ---------------------------------------------------------------------------

ensure_path() {
    local install_dir="$1"

    if [ "${SCOPE_NO_MODIFY_PATH:-0}" = "1" ]; then
        return
    fi

    # Check if already in PATH
    case ":${PATH}:" in
        *":${install_dir}:"*)
            return  # Already in PATH
            ;;
    esac

    info "Adding ${install_dir} to PATH..."

    local shell_name profile_file export_line
    shell_name="$(basename "${SHELL:-/bin/bash}")"
    export_line="export PATH=\"${install_dir}:\$PATH\""

    case "${shell_name}" in
        zsh)
            profile_file="${HOME}/.zshrc"
            ;;
        bash)
            # Prefer .bashrc for interactive shells, .bash_profile for login
            if [ -f "${HOME}/.bashrc" ]; then
                profile_file="${HOME}/.bashrc"
            else
                profile_file="${HOME}/.bash_profile"
            fi
            ;;
        fish)
            profile_file="${HOME}/.config/fish/config.fish"
            export_line="set -gx PATH ${install_dir} \$PATH"
            ;;
        *)
            profile_file="${HOME}/.profile"
            ;;
    esac

    # Don't duplicate if the line already exists
    if [ -f "${profile_file}" ] && grep -qF "${install_dir}" "${profile_file}" 2>/dev/null; then
        return
    fi

    {
        echo ""
        echo "# Added by Scope installer"
        echo "${export_line}"
    } >> "${profile_file}"

    ok "Added to ${profile_file}"
    warn "Run 'source ${profile_file}' or open a new terminal for PATH changes to take effect."
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    need_cmd curl
    need_cmd tar
    need_cmd shasum

    local suffix version tmpdir install_dir

    suffix="$(detect_platform)"
    version="$(resolve_version)"
    tmpdir="$(mktemp -d)"

    # Clean up temp directory on exit
    trap 'rm -rf "${tmpdir}"' EXIT

    download_and_verify "${version}" "${suffix}" "${tmpdir}"
    install_dir="$(install_binary "${tmpdir}")"
    ensure_path "${install_dir}"

    echo ""
    ok "Scope ${version} installed successfully!"
    echo ""
    echo "  Binary:  ${install_dir}/${BINARY_NAME}"
    echo "  Version: $(${install_dir}/${BINARY_NAME} --version 2>/dev/null || echo "${version}")"
    echo ""
    echo "  Get started:"
    echo "    scope --help"
    echo "    scope setup"
    echo ""
}

main "$@"
