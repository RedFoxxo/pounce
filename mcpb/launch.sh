#!/bin/sh
# Starts pounce from the bundle on macOS and Linux. The host runs this file
# with /bin/sh, which only needs to read it, because Claude Desktop on macOS
# extracts bundle files without the execute bit (mode 0600) and may keep the
# download's quarantine flag on them. So this script restores both on the
# program it starts, then exec replaces it: nothing stays running next to
# pounce.
dir=$(dirname "$0")
case $(uname -s)/$(uname -m) in
  Darwin/*) bin=$dir/pounce ;;
  */aarch64 | */arm64) bin=$dir/pounce-linux-arm64 ;;
  *) bin=$dir/pounce-linux-x64 ;;
esac
[ -x "$bin" ] || chmod u+x "$bin"
if [ "$(uname -s)" = Darwin ]; then
  xattr -d com.apple.quarantine "$bin" 2> /dev/null || true
fi
exec "$bin" "$@"
