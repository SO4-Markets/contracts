#!/usr/bin/env python3
"""Generate docs/build-baseline.json from optimised WASM binaries.

Run via `make wasm-baseline` after building and wasm-opt-ing the contracts.
Writes JSON to stdout; the Makefile redirects that to docs/build-baseline.json.
"""
import json
import os
import sys

WASM_DIR = "target/wasm32-unknown-unknown/release"
TEST_CONTRACTS = {"test_faucet", "test_token"}

baseline = {}

if not os.path.isdir(WASM_DIR):
    print(f"error: WASM dir not found: {WASM_DIR}", file=sys.stderr)
    print("Run 'cargo build --target wasm32-unknown-unknown --release' first.", file=sys.stderr)
    sys.exit(1)

for fname in sorted(os.listdir(WASM_DIR)):
    if not fname.endswith(".wasm"):
        continue
    name = fname[:-5]
    if name in TEST_CONTRACTS:
        continue
    size = os.path.getsize(os.path.join(WASM_DIR, fname))
    baseline[name] = size

print(json.dumps(baseline, indent=2))
