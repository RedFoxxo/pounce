#!/bin/sh
# Starts pounce from a bundle the way Claude Desktop does on macOS and Linux,
# under its worst known conditions: every file extracted as 0600 (no execute
# bit) and, on macOS, with the download's quarantine flag. Passes when the
# server answers tools/list with every tool.
#
#   mcpb/smoke.sh <pounce.mcpb>
set -eu

bundle=$1
root=$(cd "$(dirname "$0")/.." && pwd)
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
unzip -q "$bundle" -d "$dir"
find "$dir" -type f -exec chmod 600 {} +
if [ "$(uname -s)" = Darwin ]; then
  find "$dir" -type f -exec xattr -w com.apple.quarantine "0081;$(date +%s);Claude;" {} \;
fi

expected=$(jq 'length' "$root/tests/golden/tools.json")
answer=$(
  {
    printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}'
    printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
    sleep 2
  } | TP_BASE_URL=https://example.invalid TP_TOKEN=placeholder \
    /bin/sh "$dir/server/launch.sh"
)
tools=$(printf '%s\n' "$answer" | jq -s '[.[] | select(.id == 2)][0].result.tools | length')
if [ "$tools" != "$expected" ]; then
  echo "mcpb/smoke.sh: expected $expected tools, got: $answer" >&2
  exit 1
fi
echo "bundle ok: $tools tools on $(uname -s) $(uname -m)"
