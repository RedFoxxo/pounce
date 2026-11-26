#!/bin/sh
# Builds pounce.mcpb, the MCP Bundle Claude Desktop installs with one click and
# the MCP Registry lists, from a release's five binaries.
#
#   mcpb/build.sh <version> <folder with the release files> <output .mcpb>
#
# One bundle serves every system: a universal macOS binary, the Windows
# binary, and both Linux builds; launch.sh starts the right one on macOS and Linux.
set -eu

version=$1
assets=$2
out=$3
root=$(cd "$(dirname "$0")/.." && pwd)

if command -v lipo > /dev/null 2>&1; then
  lipo=lipo
elif command -v llvm-lipo > /dev/null 2>&1; then
  lipo=llvm-lipo
else
  echo "mcpb/build.sh: needs lipo (macOS) or llvm-lipo for the universal macOS binary" >&2
  exit 1
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/server"
"$lipo" -create "$assets/pounce-macos-arm64" "$assets/pounce-macos-x64" -output "$stage/server/pounce"
cp "$assets/pounce-windows-x64.exe" "$stage/server/pounce.exe"
cp "$assets/pounce-linux-x64" "$assets/pounce-linux-arm64" "$root/mcpb/launch.sh" "$stage/server/"
chmod 755 "$stage/server/pounce" "$stage/server/pounce-linux-x64" "$stage/server/pounce-linux-arm64" "$stage/server/launch.sh"
cp "$root/LICENSE" "$stage/"

# The version, and the tools as the server lists them (tests/golden/ pins that list).
jq --arg v "$version" --slurpfile golden "$root/tests/golden/tools.json" \
  '.version = $v | .tools = [$golden[0][] | { name, description }]' \
  "$root/mcpb/manifest.json" > "$stage/manifest.json"

mcpb="npx -y @anthropic-ai/mcpb@2.1.2"
$mcpb validate "$stage/manifest.json"
$mcpb pack "$stage" "$out"
