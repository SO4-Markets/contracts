#!/usr/bin/env python3
"""Resource benchmark regression gate (#1064).

Compares the JSON written by `tests/resource_bench.rs` (RESOURCE_BENCH_OUT)
against the committed baseline in docs/benchmarks/resource-baseline.json.

Subcommands:
  compare          Report before/after metrics; exit 1 on a regression past the
                   baseline's thresholds, a missing/new scenario, or a
                   protocol-bound constant raised above its recorded value.
  update           Rewrite the baseline from a current run. Requires an explicit
                   --rationale, which is stored in the baseline.
  check-rationale  CI guard: a changed baseline must carry a new rationale.

Exit codes: 0 ok, 1 gate failure, 2 usage/schema error.
"""
import argparse
import datetime
import json
import os
import re
import sys

SCHEMA_VERSION = 1
MIN_RATIONALE_LEN = 10

# Used for metrics the baseline doesn't list a threshold for.
DEFAULT_THRESHOLD = {"pct": 0.0, "abs": 0}


class GateError(Exception):
    """Usage or schema problem (exit 2)."""


def load_json(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        raise GateError(f"file not found: {path}")
    except json.JSONDecodeError as e:
        raise GateError(f"invalid JSON in {path}: {e}")


def check_schema(doc, what):
    if doc.get("schema") != SCHEMA_VERSION:
        raise GateError(
            f"{what} has schema {doc.get('schema')!r}, expected {SCHEMA_VERSION}; "
            "regenerate it with the current harness"
        )
    if not isinstance(doc.get("scenarios"), dict):
        raise GateError(f"{what} has no 'scenarios' object")


def read_constant(root, spec, name):
    """Return the integer value of `const NAME: T = <int>;` in spec['file'], or None."""
    path = os.path.join(root, spec["file"])
    try:
        with open(path, encoding="utf-8") as f:
            src = f.read()
    except FileNotFoundError:
        return None
    m = re.search(r"\bconst\s+" + re.escape(name) + r"\s*:\s*\w+\s*=\s*([0-9_]+)\s*;", src)
    return int(m.group(1).replace("_", "")) if m else None


def is_regression(base, cur, threshold):
    """True when `cur` is worse than `base` by more than both the absolute and
    percentage tolerance. Lower is always better for resource metrics."""
    delta = cur - base
    if delta <= threshold.get("abs", 0):
        return False
    if base == 0:
        return True
    return delta * 100.0 / base > threshold.get("pct", 0.0)


def compare(baseline, current, root="."):
    """Return (rows, failures). Each row: (scenario, metric, base, cur, status)."""
    check_schema(baseline, "baseline")
    check_schema(current, "current results")
    thresholds = baseline.get("thresholds", {})
    rows, failures = [], []

    base_sc, cur_sc = baseline["scenarios"], current["scenarios"]
    for name in sorted(set(base_sc) - set(cur_sc)):
        failures.append(f"scenario `{name}` is in the baseline but was not run (coverage dropped)")
    for name in sorted(set(cur_sc) - set(base_sc)):
        failures.append(
            f"scenario `{name}` has no baseline; record one with "
            "`make bench-baseline RATIONALE=...`"
        )

    for name in sorted(set(base_sc) & set(cur_sc)):
        b, c = base_sc[name], cur_sc[name]
        for metric in sorted(set(b) | set(c)):
            if metric not in b or metric not in c:
                failures.append(f"`{name}`: metric `{metric}` missing from baseline or current run")
                continue
            bv, cv = b[metric], c[metric]
            if cv == bv:
                status = "same"
            elif is_regression(bv, cv, thresholds.get(metric, DEFAULT_THRESHOLD)):
                status = "REGRESSION"
                failures.append(f"`{name}` {metric}: {bv} -> {cv} ({pct(bv, cv)})")
            elif cv > bv:
                status = "within tolerance"
            else:
                status = "improved"
            rows.append((name, metric, bv, cv, status))

    for cname, spec in sorted(baseline.get("bounded_constants", {}).items()):
        value = read_constant(root, spec, cname)
        if value is None:
            failures.append(
                f"bounded constant `{cname}` not found in {spec['file']}; if it moved or was "
                "renamed, update the baseline with evidence"
            )
        elif value > spec["value"]:
            failures.append(
                f"bounded constant `{cname}` raised {spec['value']} -> {value} without updated "
                "benchmark evidence; rerun `make bench-baseline RATIONALE=...`"
            )
    return rows, failures


def pct(base, cur):
    if base == 0:
        return "new" if cur else "0%"
    return f"{(cur - base) * 100.0 / base:+.2f}%"


def render_markdown(rows, failures, baseline):
    out = ["## Resource benchmark report", ""]
    rationale = baseline.get("meta", {}).get("rationale", "")
    if rationale:
        out += [f"Baseline rationale: _{rationale}_", ""]
    changed = [r for r in rows if r[4] != "same"]
    if failures:
        out += ["**Gate failed:**", ""] + [f"- {f}" for f in failures] + [""]
    else:
        out += ["Gate passed: no material resource regressions.", ""]
    if changed:
        out += [
            "| Scenario | Metric | Before | After | Delta | Status |",
            "|---|---|---:|---:|---:|---|",
        ]
        for name, metric, bv, cv, status in changed:
            out.append(f"| `{name}` | {metric} | {bv} | {cv} | {pct(bv, cv)} | {status} |")
    else:
        out.append("No metric changed versus the baseline.")
    out += ["", "<details><summary>All metrics</summary>", "",
            "| Scenario | Metric | Value |", "|---|---|---:|"]
    for name, metric, _bv, cv, _status in rows:
        out.append(f"| `{name}` | {metric} | {cv} |")
    out += ["", "</details>", ""]
    return "\n".join(out)


def update_baseline(baseline, current, rationale, root=".", today=None):
    if not rationale or len(rationale.strip()) < MIN_RATIONALE_LEN:
        raise GateError(
            f"a baseline update needs an explicit rationale (at least {MIN_RATIONALE_LEN} "
            "characters): make bench-baseline RATIONALE=\"why the numbers changed\""
        )
    check_schema(current, "current results")
    constants = {}
    for cname, spec in sorted(baseline.get("bounded_constants", {}).items()):
        value = read_constant(root, spec, cname)
        if value is None:
            raise GateError(f"bounded constant `{cname}` not found in {spec['file']}")
        constants[cname] = {"file": spec["file"], "value": value}
    return {
        "schema": SCHEMA_VERSION,
        "meta": {
            "rationale": rationale.strip(),
            "updated": today or datetime.date.today().isoformat(),
        },
        "thresholds": baseline.get("thresholds", {}),
        "bounded_constants": constants,
        "scenarios": current["scenarios"],
    }


def check_rationale(base, head):
    """Failures if `head` changed evidence vs `base` without a new rationale."""
    if base is None:
        return [] if head.get("meta", {}).get("rationale") else ["new baseline has no rationale"]
    keys = ("scenarios", "bounded_constants", "thresholds")
    changed = any(base.get(k) != head.get(k) for k in keys)
    same_rationale = base.get("meta", {}).get("rationale") == head.get("meta", {}).get("rationale")
    if changed and same_rationale:
        return ["baseline evidence changed but meta.rationale was not updated; "
                "regenerate it with `make bench-baseline RATIONALE=...`"]
    return []


def write_json(path, doc):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(doc, f, indent=2, sort_keys=False)
        f.write("\n")


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("compare")
    c.add_argument("--baseline", required=True)
    c.add_argument("--current", required=True)
    c.add_argument("--root", default=".")
    c.add_argument("--summary", help="also append the markdown report here (e.g. $GITHUB_STEP_SUMMARY)")

    u = sub.add_parser("update")
    u.add_argument("--baseline", required=True)
    u.add_argument("--current", required=True)
    u.add_argument("--rationale", default=os.environ.get("RATIONALE", ""))
    u.add_argument("--root", default=".")

    r = sub.add_parser("check-rationale")
    r.add_argument("--base", help="baseline from the PR base branch (omit if it did not exist)")
    r.add_argument("--head", required=True)

    args = p.parse_args(argv)
    try:
        if args.cmd == "compare":
            baseline = load_json(args.baseline)
            rows, failures = compare(baseline, load_json(args.current), args.root)
            report = render_markdown(rows, failures, baseline)
            print(report)
            if args.summary:
                with open(args.summary, "a", encoding="utf-8") as f:
                    f.write(report + "\n")
            return 1 if failures else 0
        if args.cmd == "update":
            baseline = load_json(args.baseline) if os.path.exists(args.baseline) else {}
            write_json(args.baseline, update_baseline(baseline, load_json(args.current), args.rationale, args.root))
            print(f"baseline written to {args.baseline}")
            return 0
        if args.cmd == "check-rationale":
            base = load_json(args.base) if args.base and os.path.exists(args.base) else None
            failures = check_rationale(base, load_json(args.head))
            for f in failures:
                print(f"error: {f}", file=sys.stderr)
            return 1 if failures else 0
    except GateError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    return 2


if __name__ == "__main__":
    sys.exit(main())
