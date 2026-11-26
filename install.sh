#!/bin/sh
# Installs or updates pounce on Linux and macOS:
#
#   curl -fsSL https://raw.githubusercontent.com/RedFoxxo/pounce/main/install.sh | sh
#
# Downloads the release file for this system, checks it against the release's
# SHA256SUMS, and puts it in ~/.local/bin, adding that folder to PATH in your
# shell profile when it is missing. Running it again updates pounce.
#
# Settings (environment):
#   POUNCE_VERSION          release to install, e.g. 2.0.1 (default: the latest)
#   POUNCE_INSTALL_DIR      folder for the program (default: ~/.local/bin)
#   POUNCE_NO_MODIFY_PATH   1 leaves your shell profile alone
set -eu

repo=RedFoxxo/pounce
dir=${POUNCE_INSTALL_DIR:-$HOME/.local/bin}
version=${POUNCE_VERSION:-}

fail() {
  echo "pounce install: $*" >&2
  exit 1
}

case $(uname -s) in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  *) fail "no build for $(uname -s); on Windows use install.ps1" ;;
esac
case $(uname -m) in
  x86_64 | amd64) arch=x64 ;;
  aarch64 | arm64) arch=arm64 ;;
  *) fail "no build for $(uname -m)" ;;
esac
# A shell under Rosetta reports x86_64 on Apple silicon: the native build is the better one.
if [ "$os" = macos ] && [ "$arch" = x64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
  arch=arm64
fi
asset=pounce-$os-$arch

if command -v curl > /dev/null 2>&1; then
  fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
elif command -v wget > /dev/null 2>&1; then
  fetch() { wget -q -O "$2" "$1"; }
else
  fail "needs curl or wget"
fi
if command -v sha256sum > /dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum > /dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  fail "needs sha256sum or shasum to check the download"
fi

case $version in
  '') base=https://github.com/$repo/releases/latest/download ;;
  v*) base=https://github.com/$repo/releases/download/$version ;;
  *) base=https://github.com/$repo/releases/download/v$version ;;
esac

mkdir -p "$dir"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fetch "$base/$asset" "$tmp/$asset" || fail "download of $base/$asset failed (a release just tagged may still be building)"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "download of $base/SHA256SUMS failed"
expected=$(awk -v f="$asset" '$2 == f { print $1 }' "$tmp/SHA256SUMS")
[ -n "$expected" ] && [ "$expected" = "$(sha256 "$tmp/$asset")" ] || fail "$asset does not match its SHA256SUMS entry; nothing was installed"

# Written next to the target, then renamed over it: a session running the old file keeps it until it ends.
chmod 755 "$tmp/$asset"
cp "$tmp/$asset" "$dir/.pounce.download" || fail "cannot write to $dir"
mv -f "$dir/.pounce.download" "$dir/pounce"
installed=$("$dir/pounce" --version) || fail "$dir/pounce does not start"
echo "$installed installed in $dir"

case :$PATH: in
  *":$dir:"*) exit 0 ;;
esac
line="export PATH=\"$dir:\$PATH\""
if [ "${POUNCE_NO_MODIFY_PATH:-}" = 1 ]; then
  echo "$dir is not on your PATH: add it, e.g. $line"
  exit 0
fi
# Login shells read these. bash reads ~/.bash_profile instead of ~/.profile when it exists, and
# zsh (the macOS default) reads ~/.zprofile, never ~/.profile.
changed=
for profile in "$HOME/.profile" "$HOME/.bash_profile" "$HOME/.zprofile"; do
  case $profile in
    */.bash_profile) [ -f "$profile" ] || continue ;;
    */.zprofile) [ "$os" = macos ] || [ -f "$profile" ] || continue ;;
  esac
  grep -qsF "$line" "$profile" && continue
  printf '\n# Added by the pounce installer\n%s\n' "$line" >> "$profile"
  changed="$changed ~/${profile##*/}"
done
[ -z "$changed" ] || echo "Added $dir to PATH in$changed. Log out and back in (or open a new login shell) so every program sees it."
case ${SHELL:-} in
  */fish) echo "fish: run  fish_add_path $dir  once." ;;
esac
