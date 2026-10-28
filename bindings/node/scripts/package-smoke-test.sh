#!/usr/bin/env bash
# Installs the packed npm package (plus one platform package) into an empty project, the way a
# user would, and checks every public entry point loads through both require and import.
#   scripts/package-smoke-test.sh [platform package dir containing its .node binary]
# Without an argument, the platform package is assembled from the binary `napi build --platform` left
# in the package root.
set -euo pipefail

node_dir=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [ $# -eq 1 ]; then
  platform_dir=$(cd "$1" && pwd)
else
  binaries=("$node_dir"/outpunch.*-*.node)
  if [ ${#binaries[@]} -ne 1 ] || [ ! -f "${binaries[0]}" ]; then
    echo "expected exactly one platform binary in $node_dir, found: ${binaries[*]}" >&2
    exit 1
  fi
  platform=$(basename "${binaries[0]}" .node)
  platform=${platform#outpunch.}
  platform_dir="$work/platform"
  cp -R "$node_dir/npm/$platform" "$platform_dir"
  cp "${binaries[0]}" "$platform_dir/"
fi

main_tgz=$(cd "$node_dir" && npm pack --silent --ignore-scripts --pack-destination "$work")
platform_tgz=$(cd "$platform_dir" && npm pack --silent --ignore-scripts --pack-destination "$work")

cd "$work"
npm init -y >/dev/null
npm install --silent --no-audit --no-fund "./$platform_tgz" "./$main_tgz"

node -e '
  const root = require("outpunch");
  const vite = require("outpunch/vite");
  for (const name of ["OutpunchServer", "createOutpunchMiddleware", "outpunch"]) {
    if (typeof root[name] !== "function") throw new Error(`require("outpunch").${name} missing`);
  }
  if (typeof vite.outpunch !== "function") throw new Error("require(\"outpunch/vite\").outpunch missing");
  new root.OutpunchServer({ secret: "s" });
  console.log("require: ok");
'
node --input-type=module -e '
  import { OutpunchServer, createOutpunchMiddleware } from "outpunch";
  import { outpunch } from "outpunch/vite";
  for (const f of [OutpunchServer, createOutpunchMiddleware, outpunch]) {
    if (typeof f !== "function") throw new Error("ESM export missing");
  }
  const plugin = outpunch({ secret: "s" });
  if (plugin.name !== "outpunch") throw new Error("vite plugin malformed");
  console.log("import: ok");
'
