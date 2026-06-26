#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf '%s\n' "Usage: $0 [--baseline <file>] [--fail-on-increase]"
}

baseline=""
fail_on_increase="0"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --baseline)
      if [ "$#" -lt 2 ]; then
        usage >&2
        exit 2
      fi
      baseline="$2"
      shift 2
      ;;
    --fail-on-increase)
      fail_on_increase="1"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

if ! command -v stellar >/dev/null 2>&1; then
  printf '%s\n' "error: stellar CLI is required" >&2
  exit 1
fi

if ! command -v wasm-opt >/dev/null 2>&1; then
  printf '%s\n' "error: wasm-opt is required" >&2
  exit 1
fi

json_tool=""
if command -v python3 >/dev/null 2>&1 && python3 -c 'import json' >/dev/null 2>&1; then
  json_tool="python3"
elif command -v python >/dev/null 2>&1 && python -c 'import json' >/dev/null 2>&1; then
  json_tool="python"
elif command -v node >/dev/null 2>&1 && node -e 'JSON.parse("{}")' >/dev/null 2>&1; then
  json_tool="node"
else
  printf '%s\n' "error: python3, python, or node is required" >&2
  exit 1
fi

if [ -n "$baseline" ] && [ ! -f "$baseline" ]; then
  printf 'error: baseline file not found: %s\n' "$baseline" >&2
  exit 1
fi

release_dir="target/wasm32-unknown-unknown/release"
mkdir -p "$release_dir"
rm -f "$release_dir"/*.wasm

if stellar contract build --help | grep -q -- '--release'; then
  stellar contract build --release --out-dir "$release_dir"
else
  stellar contract build --profile release --out-dir "$release_dir"
fi

optimized_dir="$release_dir/optimized"
mkdir -p "$optimized_dir" target

shopt -s nullglob
wasm_files=("$release_dir"/*.wasm)
if [ "${#wasm_files[@]}" -eq 0 ]; then
  printf 'error: no WASM files found in %s\n' "$release_dir" >&2
  exit 1
fi

rm -f "$optimized_dir"/*.wasm
for wasm in "${wasm_files[@]}"; do
  wasm-opt -O3 "$wasm" -o "$optimized_dir/$(basename "$wasm")"
done

optimized_files=("$optimized_dir"/*.wasm)
if [ "$json_tool" = "node" ]; then
  node - "$baseline" "$fail_on_increase" "target/sizes.json" "${optimized_files[@]}" <<'JS'
const fs = require("fs");
const path = require("path");

const baselinePath = process.argv[2];
const failOnIncrease = process.argv[3] === "1";
const outputPath = process.argv[4];
const wasmPaths = process.argv.slice(5);

let baseline = null;
if (baselinePath) {
  baseline = JSON.parse(fs.readFileSync(baselinePath, "utf8"));
}

const sizes = {};
for (const wasmPath of wasmPaths) {
  sizes[path.basename(wasmPath, ".wasm")] = fs.statSync(wasmPath).size;
}

const sortedSizes = Object.fromEntries(
  Object.entries(sizes).sort(([left], [right]) => left.localeCompare(right))
);
fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(sortedSizes, null, 2)}\n`);

function deltaText(contract, size) {
  if (baseline === null) {
    return "-";
  }
  const oldSize = baseline[contract];
  if (oldSize === undefined) {
    return "n/a";
  }
  const diff = size - Number(oldSize);
  const pct = Number(oldSize) === 0 ? "n/a" : `${((diff / Number(oldSize)) * 100).toFixed(2).replace(/^(?!-)/, "+")}%`;
  return `${diff >= 0 ? "+" : ""}${diff} (${pct})`;
}

console.log("| Contract | WASM size (bytes) | Δ from baseline |");
console.log("| --- | ---: | ---: |");
const increases = [];
for (const [contract, size] of Object.entries(sortedSizes)) {
  console.log(`| ${contract} | ${size} | ${deltaText(contract, size)} |`);
  if (baseline !== null && baseline[contract] !== undefined) {
    const diff = size - Number(baseline[contract]);
    if (diff > 0) {
      increases.push(`${contract}: +${diff} bytes`);
    }
  }
}

if (failOnIncrease && increases.length > 0) {
  console.error("");
  console.error("WASM size increased against baseline:");
  for (const increase of increases) {
    console.error(`- ${increase}`);
  }
  process.exit(1);
}
JS
else
  "$json_tool" - "$baseline" "$fail_on_increase" "target/sizes.json" "${optimized_files[@]}" <<'PY'
import json
import os
import sys

baseline_path = sys.argv[1]
fail_on_increase = sys.argv[2] == "1"
output_path = sys.argv[3]
wasm_paths = sys.argv[4:]

baseline = None
if baseline_path:
    with open(baseline_path, "r", encoding="utf-8") as fh:
        baseline = json.load(fh)

sizes = {
    os.path.splitext(os.path.basename(path))[0]: os.path.getsize(path)
    for path in wasm_paths
}

os.makedirs(os.path.dirname(output_path), exist_ok=True)
with open(output_path, "w", encoding="utf-8") as fh:
    json.dump(dict(sorted(sizes.items())), fh, indent=2)
    fh.write("\n")

def delta_text(contract, size):
    if baseline is None:
        return "-"
    old_size = baseline.get(contract)
    if old_size is None:
        return "n/a"
    diff = size - int(old_size)
    if old_size == 0:
        pct = "n/a"
    else:
        pct = f"{(diff / int(old_size)) * 100:+.2f}%"
    return f"{diff:+d} ({pct})"

print("| Contract | WASM size (bytes) | Δ from baseline |")
print("| --- | ---: | ---: |")
increases = []
for contract, size in sorted(sizes.items()):
    print(f"| {contract} | {size} | {delta_text(contract, size)} |")
    if baseline is not None and contract in baseline:
        diff = size - int(baseline[contract])
        if diff > 0:
            increases.append(f"{contract}: +{diff} bytes")

if fail_on_increase and increases:
    print("", file=sys.stderr)
    print("WASM size increased against baseline:", file=sys.stderr)
    for increase in increases:
        print(f"- {increase}", file=sys.stderr)
    sys.exit(1)
PY
fi
