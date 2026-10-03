#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo 'usage: smoke-packaged.sh <native-package.tgz> <rust-target> [dispatcher-package.tgz]' >&2
  exit 2
fi

tarball=$(realpath "$1")
target=$2
case "$target" in
  x86_64-unknown-linux-gnu) name=work-linux-x64-gnu ;;
  aarch64-unknown-linux-gnu) name=work-linux-arm64-gnu ;;
  aarch64-apple-darwin) name=work-darwin-arm64 ;;
  *) echo "unsupported target: $target" >&2; exit 2 ;;
esac

scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
packages=("$tarball")
if [[ $# == 3 ]]; then packages+=("$(realpath "$3")"); fi
npm install --prefix "$scratch/install" --offline --ignore-scripts --no-audit --no-fund --no-save --omit=optional "${packages[@]}" >/dev/null
package="$scratch/install/node_modules/@convesoft/$name"
binary="$package/bin/work"
test -x "$binary"
for file in LICENSE LICENSE-MIT LICENSE-APACHE README.md; do test -s "$package/$file"; done
if [[ $# == 3 ]]; then
  dispatcher="$scratch/install/node_modules/@convesoft/work"
  for file in LICENSE LICENSE-MIT LICENSE-APACHE README.md; do test -s "$dispatcher/$file"; done
  test "$(node -p 'require(process.argv[1]).license' "$dispatcher/package.json")" = 'MIT OR Apache-2.0'
  binary="$scratch/install/node_modules/.bin/work"
  test -x "$binary"
fi

version=$(node -p 'require(process.argv[1]).version' "$package/package.json")
license=$(node -p 'require(process.argv[1]).license' "$package/package.json")
test "$license" = 'MIT OR Apache-2.0'
test "$("$binary" --version)" = "work $version"

fixture="$scratch/project"
mkdir -p "$fixture/.work/items"
git -C "$fixture" init --quiet
created=$(cd "$fixture" && "$binary" --json item create --title 'Packaged smoke item' --body 'Packaged CLI and MCP smoke test')
item_id=$(node -e '
  const result = JSON.parse(process.argv[1]);
  if (result.ok !== true || !/^[0-9a-f]{32}$/.test(result.result?.item?.id)) process.exit(1);
  process.stdout.write(result.result.item.id);
' "$created")
(cd "$fixture" && "$binary" --json item ready) | node -e '
  let source = "";
  process.stdin.on("data", chunk => source += chunk);
  process.stdin.on("end", () => {
    const result = JSON.parse(source);
    if (result.ok !== true || !Array.isArray(result.result?.items) || result.result.items.length !== 1) process.exit(1);
  });
'

node - "$binary" "$fixture" "$item_id" <<'NODE'
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const [binary, fixture, itemId] = process.argv.slice(2);
const requests = [
  { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "packaged-smoke", version: "1" } } },
  { jsonrpc: "2.0", method: "notifications/initialized", params: {} },
  { jsonrpc: "2.0", id: 2, method: "tools/list", params: {} },
  { jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "item_inspect", arguments: { id: itemId } } },
];
(async () => {
  const child = spawn(binary, ["mcp"], { cwd: fixture, stdio: ["pipe", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", chunk => stdout += chunk);
  child.stderr.setEncoding("utf8").on("data", chunk => stderr += chunk);
  const timer = setTimeout(() => child.kill("SIGKILL"), 10000);
  child.stdin.end(requests.map(request => JSON.stringify(request)).join("\n") + "\n");
  const status = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", resolve);
  });
  clearTimeout(timer);
  assert.equal(status, 0, stderr);
  const responses = stdout.trim().split("\n").map(line => JSON.parse(line));
  const byId = new Map(responses.map(response => [response.id, response]));
  assert.equal(byId.get(1)?.result?.protocolVersion, "2025-06-18");
  assert.ok(byId.get(2)?.result?.tools?.some(tool => tool.name === "item_inspect"));
  assert.equal(byId.get(3)?.result?.structuredContent?.item?.id, itemId);
})().catch(error => { console.error(error); process.exitCode = 1; });
NODE

node "$(dirname "${BASH_SOURCE[0]}")/verify-beta.mjs" "$binary"

echo "packaged CLI/MCP smoke passed: @convesoft/$name@$version ($target)${dispatcher:+ via @convesoft/work}"
