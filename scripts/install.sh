#!/usr/bin/env bash
set -euo pipefail

REPO="pieza/iris-tv"
ASSET="iris-aarch64-unknown-linux-gnu.tar.gz"
VERSION="${1:-latest}"

if [[ "$(uname -m)" != "aarch64" && "$(uname -m)" != "arm64" ]]; then
  echo "IRIS release packages currently target Raspberry Pi OS arm64/aarch64." >&2
  echo "Detected architecture: $(uname -m)" >&2
  exit 1
fi

for tool in curl tar install; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Missing required command: $tool" >&2
    exit 1
  fi
done

if [[ "$VERSION" == "latest" ]]; then
  URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
else
  if [[ "$VERSION" != V* && "$VERSION" != v* ]]; then
    VERSION="V${VERSION}"
  fi
  URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"
fi

SUDO=""
if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "This installer needs root permissions. Re-run as root or install sudo." >&2
    exit 1
  fi
  SUDO="sudo"
fi

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "Downloading IRIS from ${URL}"
curl -fsSL "$URL" -o "${TMP_DIR}/${ASSET}"
tar -xzf "${TMP_DIR}/${ASSET}" -C "$TMP_DIR"

if [[ ! -x "${TMP_DIR}/iris/iris" ]]; then
  echo "Downloaded package does not contain an executable iris binary." >&2
  exit 1
fi

if [[ ! -d "${TMP_DIR}/iris/profiles" ]]; then
  echo "Downloaded package does not contain profiles/." >&2
  exit 1
fi

$SUDO install -Dm755 "${TMP_DIR}/iris/iris" /usr/local/bin/iris
$SUDO mkdir -p /usr/local/share/iris
$SUDO rm -rf /usr/local/share/iris/profiles
$SUDO cp -R "${TMP_DIR}/iris/profiles" /usr/local/share/iris/profiles

echo "IRIS installed to /usr/local/bin/iris"
echo "Profiles installed to /usr/local/share/iris/profiles"
echo "Try: iris start telstar"
