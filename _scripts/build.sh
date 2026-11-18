#!/usr/bin/env bash

# Make bash fail on first error, unset variables, and pipe failures
set -o errexit -o nounset -o pipefail

# Set field separators to newlines and tabs only, for safer word splitting in loops
IFS=$'\n\t'

if [[ $# -ne 1 ]]; then
  echo 'usage: build.sh <site name>' >&2
  exit 1
fi

SITE_NAME="$1"

if [[ $(uname) == "Linux" ]]; then
  cat /proc/version;
else
  uname -v;
fi

RELEASES="https://github.com/chriskrycho/web/releases"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

# Render persists $XDG_CACHE_HOME between builds of a service, so caching by
# release tag saves the download without ever reusing a different build of lx.
# Set LX_RELEASE_TAG to pin a release instead of following `latest`.
CACHE_ROOT="${XDG_CACHE_HOME:-${HOME}/.cache}/lx"
BINARY="${WORK_DIR}/lx-cli"

latest_tag() {
  curl --head --location --silent --show-error \
      --proto '=https' --tlsv1.2 \
      --max-redirs 5 \
      "${RELEASES}/latest/download/lx-linux" \
    | tr -d '\r' \
    | sed -n 's#^[Ll]ocation: .*/releases/download/\([^/]*\)/lx-linux.*#\1#p' \
    | tail -n 1
}

if [[ -n "${LX_RELEASE_TAG:-}" ]]; then
  TAG="$LX_RELEASE_TAG"
  ASSET_URL="${RELEASES}/download/${TAG}/lx-linux"
else
  TAG="$(latest_tag)" || TAG=""
  ASSET_URL="${RELEASES}/latest/download/lx-linux"
fi

# Never let an unexpected value escape into the cache path.
if [[ ! "$TAG" =~ ^[A-Za-z0-9._-]+$ ]]; then
  TAG=""
fi

if [[ -n "$TAG" && -x "${CACHE_ROOT}/${TAG}/lx-linux" ]]; then
  echo "using cached lx release '$TAG'"
  cp "${CACHE_ROOT}/${TAG}/lx-linux" "$BINARY"
else
  echo "fetching '$ASSET_URL' to '$BINARY'"
  curl --location \
      --proto '=https' --tlsv1.2 \
      --silent --show-error --fail \
      --output "$BINARY" \
      "$ASSET_URL"
  chmod +x "$BINARY"

  if [[ -n "$TAG" ]]; then
    mkdir -p "${CACHE_ROOT}/${TAG}"
    cp "$BINARY" "${CACHE_ROOT}/${TAG}/lx-linux"
  fi
fi

echo "building '$SITE_NAME'"
"$BINARY" publish "./sites/${SITE_NAME}"
