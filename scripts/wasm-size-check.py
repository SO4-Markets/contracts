#!/usr/bin/env python3
"""Report contract WASM sizes and block growth past the budget.

Usage: wasm-size-check.py HEAD_DIR [BASE_DIR]

Both directories hold optimised release *.wasm files. Without BASE_DIR (a push
to main) the sizes are only reported. Exits 1 when any contract grows by more
than BLOCK_THRESHOLD over the base.
"""

import os
import sys

# Test fixtures are never deployed, so they are exempt from the size budget.
TEST_CONTRACTS = {"test_faucet", "test_token"}

WARN_THRESHOLD = 0.05   # 5%
BLOCK_THRESHOLD = 0.10  # 10%


def contract_sizes(directory):
    sizes = {}
    for fname in os.listdir(directory):
        name, ext = os.path.splitext(fname)
        if ext == ".wasm" and name not in TEST_CONTRACTS:
            sizes[name] = os.path.getsize(os.path.join(directory, fname))
    return sizes


def report_row(name, head_size, base_size):
    if base_size is None:
        return f"| `{name}` | — | {head_size:,} | — | — |"
    ratio = (head_size - base_size) / base_size
    if ratio > BLOCK_THRESHOLD:
        status = "🔴 BLOCK"
    elif ratio > WARN_THRESHOLD:
        status = "🟡 WARN"
    else:
        status = "✅"
    return f"| `{name}` | {base_size:,} | {head_size:,} | {ratio:+.1%} | {status} |"


def main(argv):
    head = contract_sizes(argv[1])
    base = contract_sizes(argv[2]) if len(argv) > 2 else {}

    rows = ["| Contract | Base (bytes) | PR (bytes) | Change | Status |", "|---|---|---|---|---|"]
    blocked = False
    for name in sorted(head):
        base_size = base.get(name) if base else None
        rows.append(report_row(name, head[name], base_size))
        if base_size is not None and (head[name] - base_size) / base_size > BLOCK_THRESHOLD:
            blocked = True

    report = "\n".join(rows)
    print(report)

    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a") as summary:
            summary.write(report + "\n")

    return 1 if blocked else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
