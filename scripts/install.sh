#!/bin/sh
# Moss installer: downloads a release tarball, verifies its SHA-256 against SHA256SUMS.txt and installs the binary.
# It installs one file (`moss`) and runs nothing else. Read it before piping it into a shell.
#
#   curl -fsSL https://github.com/T1nkiePlayz/Moss/releases/latest/download/install.sh | sh
#   sh install.sh --version v0.2.0 --prefix "$HOME/.local"
#
# Options
#   --version TAG   install this release (default: the latest published, non-draft release)
#   --prefix DIR    install to DIR/bin (default /usr/local/bin, using sudo when it is not writable)
#   -h, --help      show this help
# Environment
#   MOSS_VERSION        same as --version
#   MOSS_REPO           owner/name on GitHub (default T1nkiePlayz/Moss)
#   GITHUB_TOKEN        needed while the repository is private (uses the API to fetch the assets)
#   MOSS_SKIP_COSIGN    set to 1 to skip the optional signature check (done only when cosign is installed)
#   MOSS_RELEASE_BASE   alternative download root laid out as <base>/<tag>/<asset> (plus <base>/LATEST naming the
#                       newest tag); used for mirrors and tests. Accepts http(s):// and file:// URLs.
set -eu

REPO="${MOSS_REPO:-T1nkiePlayz/Moss}"
VERSION="${MOSS_VERSION:-}"
PREFIX=""
BASE="${MOSS_RELEASE_BASE:-}"
API="https://api.github.com/repos/$REPO"

say() { printf '%s\n' "$*"; }
die() { printf 'moss-install: %s\n' "$*" >&2; exit 1; }
usage() {
  cat <<'EOT'
Usage: install.sh [--version TAG] [--prefix DIR]
  --version TAG   install this release (default: the latest published release)
  --prefix DIR    install to DIR/bin (default /usr/local/bin, using sudo when it is not writable)
Environment: MOSS_VERSION, MOSS_REPO, GITHUB_TOKEN (private repo), MOSS_RELEASE_BASE (mirror or test directory),
  MOSS_SKIP_COSIGN=1 (skip the optional cosign signature check).
EOT
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version) [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift 2 ;;
    --version=*) VERSION="${1#--version=}"; shift ;;
    --prefix) [ $# -ge 2 ] || die "--prefix needs a value"; PREFIX="$2"; shift 2 ;;
    --prefix=*) PREFIX="${1#--prefix=}"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
done

# --- platform ---------------------------------------------------------------------------------------------------
case "$(uname -s)" in
  Linux) OS=linux ;;
  Darwin) OS=macos ;;
  *) die "unsupported operating system $(uname -s); Moss supports Linux and macOS" ;;
esac
case "$(uname -m)" in
  x86_64|amd64) ARCH=x86_64 ;;
  aarch64|arm64) ARCH=aarch64 ;;
  *) die "unsupported CPU architecture $(uname -m)" ;;
esac
if [ "$OS" = linux ] && [ "$ARCH" != x86_64 ]; then
  die "there is no prebuilt Linux $ARCH release yet; build from source (cargo build --release) or use the AUR package"
fi

# --- tools ------------------------------------------------------------------------------------------------------
if command -v curl >/dev/null 2>&1; then DL=curl
elif command -v wget >/dev/null 2>&1; then DL=wget
else die "curl or wget is required"; fi
if command -v sha256sum >/dev/null 2>&1; then SUM="sha256sum"
elif command -v shasum >/dev/null 2>&1; then SUM="shasum -a 256"
else die "sha256sum or shasum is required to verify the download"; fi

# fetch URL DEST [accept-header]: GitHub's token is sent only to api.github.com
fetch() {
  _url="$1"; _out="$2"; _accept="${3:-}"
  case "$DL" in
    curl)
      set -- -fsSL --retry 3 -o "$_out"
      [ -z "$_accept" ] || set -- "$@" -H "Accept: $_accept"
      if [ -n "${GITHUB_TOKEN:-}" ]; then case "$_url" in https://api.github.com/*) set -- "$@" -H "Authorization: Bearer $GITHUB_TOKEN" ;; esac; fi
      curl "$@" "$_url" ;;
    wget)
      set -- -q -O "$_out"
      [ -z "$_accept" ] || set -- "$@" --header="Accept: $_accept"
      if [ -n "${GITHUB_TOKEN:-}" ]; then case "$_url" in https://api.github.com/*) set -- "$@" --header="Authorization: Bearer $GITHUB_TOKEN" ;; esac; fi
      wget "$@" "$_url" ;;
  esac
}

TMP="$(mktemp -d 2>/dev/null || mktemp -d -t moss-install)"
trap 'rm -rf "$TMP"' EXIT INT TERM

# --- which release ----------------------------------------------------------------------------------------------
if [ -z "$VERSION" ]; then
  if [ -n "$BASE" ]; then
    fetch "$BASE/LATEST" "$TMP/latest" || die "could not read $BASE/LATEST"
    VERSION="$(tr -d ' \r\n' < "$TMP/latest")"
  else
    fetch "$API/releases/latest" "$TMP/latest.json" "application/vnd.github+json" \
      || die "could not look up the latest release (a private repository needs GITHUB_TOKEN; drafts and pre-releases need --version)"
    VERSION="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$TMP/latest.json" | head -n 1)"
  fi
  [ -n "$VERSION" ] || die "could not work out the latest version; pass --version vX.Y.Z"
fi
case "$VERSION" in v*) ;; *) VERSION="v$VERSION" ;; esac
printf '%s' "$VERSION" | grep -Eq '^v[0-9A-Za-z._+-]+$' || die "invalid version: $VERSION"

NAME="moss-$VERSION-$OS-$ARCH"
TARBALL="$NAME.tar.gz"
say "Installing Moss $VERSION for $OS-$ARCH"

# --- download ---------------------------------------------------------------------------------------------------
# asset_api_url NAME: the API URL of a release asset (private repositories do not serve browser_download_url to tokens).
asset_api_url() {
  tr '{},' '\n\n\n' < "$TMP/release.json" | awk -v want="$1" '
    /"url"[[:space:]]*:[[:space:]]*"[^"]*\/releases\/assets\/[0-9]+"/ { u=$0; sub(/.*"url"[[:space:]]*:[[:space:]]*"/, "", u); sub(/".*/, "", u) }
    /"name"[[:space:]]*:/ { n=$0; sub(/.*"name"[[:space:]]*:[[:space:]]*"/, "", n); sub(/".*/, "", n); if (n == want && u != "") { print u; exit } }
  '
}

get_asset() {
  if [ -n "$BASE" ]; then
    fetch "$BASE/$VERSION/$1" "$TMP/$1" || die "download failed: $BASE/$VERSION/$1"
  elif [ -n "${GITHUB_TOKEN:-}" ]; then
    [ -f "$TMP/release.json" ] || fetch "$API/releases/tags/$VERSION" "$TMP/release.json" "application/vnd.github+json" || die "release $VERSION not found (or the token cannot see it)"
    _u="$(asset_api_url "$1")"; [ -n "$_u" ] || die "release $VERSION has no asset $1"
    fetch "$_u" "$TMP/$1" "application/octet-stream" || die "download failed: $1"
  else
    fetch "https://github.com/$REPO/releases/download/$VERSION/$1" "$TMP/$1" || die "download failed: $1 (a private repository needs GITHUB_TOKEN)"
  fi
}

get_asset "$TARBALL"
get_asset "SHA256SUMS.txt"

# --- verify -----------------------------------------------------------------------------------------------------
# Optional: when cosign is installed and the release carries a signature bundle, check that SHA256SUMS.txt was signed
# by this repository's release workflow (Sigstore keyless). cosign is never required; a release without a bundle
# (older releases, private repositories) is installed on the checksum alone. A bundle that does not verify is fatal.
if [ "${MOSS_SKIP_COSIGN:-}" = 1 ]; then
  say "Skipping the signature check (MOSS_SKIP_COSIGN=1)"
elif command -v cosign >/dev/null 2>&1; then
  if ( get_asset "SHA256SUMS.txt.sigstore.json" ) >/dev/null 2>&1; then
    _repo_re="$(printf '%s' "$REPO" | sed 's/[.[\*^$+?(){}|]/\\&/g')"
    cosign verify-blob --bundle "$TMP/SHA256SUMS.txt.sigstore.json" \
      --certificate-identity-regexp "^https://github\.com/$_repo_re/\.github/workflows/release\.yml@refs/(tags/v.+|heads/.+)\$" \
      --certificate-oidc-issuer https://token.actions.githubusercontent.com \
      "$TMP/SHA256SUMS.txt" >/dev/null 2>&1 \
      || die "the signature on SHA256SUMS.txt did not verify; nothing was installed (run cosign verify-blob by hand to see why, or set MOSS_SKIP_COSIGN=1 to skip this check)"
    say "Signature OK (cosign, signed by the $REPO release workflow)"
  else
    say "No signature bundle in this release; relying on the checksum only"
  fi
else
  say "cosign is not installed; skipping the optional signature check (checksum is still verified)"
fi

want="$(awk -v f="$TARBALL" '{ n=$2; sub(/^\*/, "", n); if (n == f) { print $1; exit } }' "$TMP/SHA256SUMS.txt")"
[ -n "$want" ] || die "SHA256SUMS.txt has no entry for $TARBALL"
have="$($SUM "$TMP/$TARBALL" | awk '{print $1}')"
[ "$want" = "$have" ] || die "checksum mismatch for $TARBALL (expected $want, got $have); nothing was installed"
say "Checksum OK ($have)"

# --- install ----------------------------------------------------------------------------------------------------
tar -xzf "$TMP/$TARBALL" -C "$TMP" "$NAME/moss" || die "the tarball does not contain $NAME/moss"
if [ -n "$PREFIX" ]; then DEST="$PREFIX/bin"; else DEST="/usr/local/bin"; fi
SUDO=""
if ! { mkdir -p "$DEST" 2>/dev/null && [ -w "$DEST" ]; }; then
  [ "$(id -u)" -eq 0 ] && die "cannot write to $DEST"
  command -v sudo >/dev/null 2>&1 || die "$DEST is not writable and sudo is not available; rerun with --prefix \"\$HOME/.local\""
  SUDO="sudo"
  say "$DEST needs elevated rights; using sudo for the copy only"
  $SUDO mkdir -p "$DEST"
fi
$SUDO install -m 0755 "$TMP/$NAME/moss" "$DEST/moss"
say "Installed $DEST/moss"

# --- next steps -------------------------------------------------------------------------------------------------
say ""
say "Next steps"
say "  Start it (sign-in is built in):           mkdir -p ~/moss && cd ~/moss && $DEST/moss serve"
say "  Then open http://127.0.0.1:8420, sign in, and enter the setup token from ~/moss/moss-data/setup-token."
say "  Try it without sign-in (loopback only):   MOSS_AUTH=dev $DEST/moss serve"
say "  Check the environment:                    $DEST/moss doctor"
if [ "$OS" = linux ]; then
  say "  Run as a service: the tarball ships moss.service, .env and moss.env.example (see the README, Linux: production-style install)."
else
  say "  Run at login: the tarball ships io.moss.controller.plist; see docs/macos.md (launchd), or use Homebrew's moss.rb."
  say "  If macOS blocks the binary: xattr -d com.apple.quarantine $DEST/moss"
fi
case ":$PATH:" in *":$DEST:"*) ;; *) say "  Note: $DEST is not on your PATH." ;; esac
