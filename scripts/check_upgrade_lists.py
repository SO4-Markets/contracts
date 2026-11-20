#!/usr/bin/env python3
"""
check_upgrade_lists.py — Assert that upgrade.mk's contract lists match source.

Reads the three lists from mx/upgrade.mk:

    UPGRADE_CONTRACTS      — crates that expose `pub fn upgrade`
    IMMUTABLE_CONTRACTS    — crates explicitly marked as never-upgradeable
    NON_UPGRADEABLE_CONTRACTS — crates that currently lack upgrade() (tracked
                               separately; presence in source, absence in fn)

Then scans contracts/*/src/lib.rs for the presence of `pub fn upgrade`, and
asserts:

  1. Every crate in UPGRADE_CONTRACTS has `pub fn upgrade` in its lib.rs.
  2. No crate in IMMUTABLE_CONTRACTS or NON_UPGRADEABLE_CONTRACTS has
     `pub fn upgrade` in its lib.rs.
  3. Every crate in contracts/ that has `pub fn upgrade` appears in
     UPGRADE_CONTRACTS (i.e. the list is complete — nothing is silently
     upgradeable without being listed).
  4. The three lists are mutually exclusive (no crate appears in two lists).
  5. The union of the three lists equals the full set of contract crates
     (no contract is omitted from every list).

Usage:
    python3 scripts/check_upgrade_lists.py           # print report, exit 0/1
    python3 scripts/check_upgrade_lists.py --fix-hint # also print the corrected
                                                       # lists for copy-paste
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# ── Paths ────────────────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).resolve().parent.parent
CONTRACTS_DIR = REPO_ROOT / "contracts"
UPGRADE_MK = REPO_ROOT / "mx" / "upgrade.mk"

# Contract directories whose names begin with "test_" are excluded because
# they are dev-only helpers, not deployable protocol contracts.
EXCLUDED_CRATES = {"test_faucet", "test_token"}

# ── Makefile parser ──────────────────────────────────────────────────────────

_MK_LIST_RE = re.compile(
    r"^([A-Z_]+)\s*[:?]?=\s*(.*?)(?=^[A-Z_]+\s*[:?]?=|\Z)",
    re.MULTILINE | re.DOTALL,
)
_CONTINUATION_RE = re.compile(r"\\\n")


def _strip_comments(text: str) -> str:
    return re.sub(r"#.*", "", text)


def parse_mk_list(variable: str, mk_text: str) -> list[str]:
    """Extract a multi-line Makefile variable value as a sorted list of tokens."""
    cleaned = _strip_comments(mk_text)
    for m in _MK_LIST_RE.finditer(cleaned):
        if m.group(1).strip() == variable:
            raw = _CONTINUATION_RE.sub(" ", m.group(2))
            return [t.strip() for t in raw.split() if t.strip()]
    return []


# ── Source scanner ───────────────────────────────────────────────────────────

_UPGRADE_FN_RE = re.compile(r"pub\s+fn\s+upgrade\s*\(")


def crates_with_upgrade(contracts_dir: Path) -> set[str]:
    """Return the set of crate names whose lib.rs defines `pub fn upgrade`."""
    found: set[str] = set()
    for lib_rs in sorted(contracts_dir.glob("*/src/lib.rs")):
        crate = lib_rs.parent.parent.name
        if crate in EXCLUDED_CRATES:
            continue
        source = lib_rs.read_text()
        if _UPGRADE_FN_RE.search(source):
            found.add(crate)
    return found


def all_contract_crates(contracts_dir: Path) -> set[str]:
    """Return all crate names under contracts/, excluding test helpers."""
    return {
        d.name
        for d in contracts_dir.iterdir()
        if d.is_dir() and (d / "src" / "lib.rs").exists()
        and d.name not in EXCLUDED_CRATES
    }


# ── Checks ───────────────────────────────────────────────────────────────────

def run_checks(fix_hint: bool = False) -> bool:
    """Run all checks. Return True if all pass, False otherwise."""
    if not UPGRADE_MK.exists():
        print(f"ERROR: {UPGRADE_MK} not found.")
        return False

    mk_text = UPGRADE_MK.read_text()

    upgrade_list = set(parse_mk_list("UPGRADE_CONTRACTS", mk_text))
    immutable_list = set(parse_mk_list("IMMUTABLE_CONTRACTS", mk_text))
    non_upgradeable_list = set(parse_mk_list("NON_UPGRADEABLE_CONTRACTS", mk_text))

    source_has_upgrade = crates_with_upgrade(CONTRACTS_DIR)
    all_crates = all_contract_crates(CONTRACTS_DIR)

    errors: list[str] = []
    warnings: list[str] = []

    # ── Check 1: Every crate in UPGRADE_CONTRACTS has pub fn upgrade ─────────
    for crate in sorted(upgrade_list):
        lib = CONTRACTS_DIR / crate / "src" / "lib.rs"
        if not lib.exists():
            errors.append(
                f"UPGRADE_CONTRACTS lists '{crate}' but contracts/{crate}/src/lib.rs does not exist"
            )
        elif crate not in source_has_upgrade:
            errors.append(
                f"UPGRADE_CONTRACTS lists '{crate}' but `pub fn upgrade` not found in its lib.rs"
            )

    # ── Check 2: No immutable/non-upgradeable crate has pub fn upgrade ───────
    for crate in sorted(immutable_list):
        if crate in source_has_upgrade:
            errors.append(
                f"IMMUTABLE_CONTRACTS lists '{crate}' but it HAS `pub fn upgrade` in source — "
                f"move it to UPGRADE_CONTRACTS or remove the upgrade entrypoint"
            )
    for crate in sorted(non_upgradeable_list):
        if crate in source_has_upgrade:
            errors.append(
                f"NON_UPGRADEABLE_CONTRACTS lists '{crate}' but it HAS `pub fn upgrade` in source — "
                f"move it to UPGRADE_CONTRACTS or remove the upgrade entrypoint"
            )

    # ── Check 3: Every crate with pub fn upgrade is in UPGRADE_CONTRACTS ─────
    unlisted = source_has_upgrade - upgrade_list
    for crate in sorted(unlisted):
        errors.append(
            f"contracts/{crate}/src/lib.rs defines `pub fn upgrade` "
            f"but '{crate}' is NOT in UPGRADE_CONTRACTS"
        )

    # ── Check 4: Lists are mutually exclusive ─────────────────────────────────
    overlap_ui = upgrade_list & immutable_list
    overlap_un = upgrade_list & non_upgradeable_list
    overlap_in = immutable_list & non_upgradeable_list
    for crate in sorted(overlap_ui):
        errors.append(f"'{crate}' appears in both UPGRADE_CONTRACTS and IMMUTABLE_CONTRACTS")
    for crate in sorted(overlap_un):
        errors.append(f"'{crate}' appears in both UPGRADE_CONTRACTS and NON_UPGRADEABLE_CONTRACTS")
    for crate in sorted(overlap_in):
        errors.append(f"'{crate}' appears in both IMMUTABLE_CONTRACTS and NON_UPGRADEABLE_CONTRACTS")

    # ── Check 5: Every contract crate is in exactly one list ─────────────────
    declared = upgrade_list | immutable_list | non_upgradeable_list
    not_declared = all_crates - declared
    for crate in sorted(not_declared):
        errors.append(
            f"contracts/{crate} is not listed in any of UPGRADE_CONTRACTS, "
            f"IMMUTABLE_CONTRACTS, or NON_UPGRADEABLE_CONTRACTS — "
            f"add it to the appropriate list in mx/upgrade.mk"
        )

    # ── Report ────────────────────────────────────────────────────────────────
    if errors:
        print(f"upgrade-list check FAILED — {len(errors)} error(s):\n")
        for e in errors:
            print(f"  ✗ {e}")

        if fix_hint:
            print("\n── Suggested corrected lists (copy into mx/upgrade.mk) ──────────────────")
            actual_upgrade = sorted(source_has_upgrade)
            actual_immutable = sorted(immutable_list - source_has_upgrade)
            uncategorised = sorted(not_declared - source_has_upgrade)
            print(f"\nUPGRADE_CONTRACTS := \\\n\t" + " \\\n\t".join(actual_upgrade))
            if actual_immutable:
                print(f"\nIMMUTABLE_CONTRACTS := \\\n\t" + " \\\n\t".join(actual_immutable))
            if uncategorised:
                print(
                    f"\n# These crates have no upgrade(); assign to IMMUTABLE_CONTRACTS or\n"
                    f"# NON_UPGRADEABLE_CONTRACTS as appropriate:\n# "
                    + "\n# ".join(uncategorised)
                )
        return False

    if warnings:
        for w in warnings:
            print(f"  ⚠ {w}")

    print(
        f"upgrade-list check PASSED — "
        f"UPGRADE_CONTRACTS={len(upgrade_list)}, "
        f"IMMUTABLE_CONTRACTS={len(immutable_list)}, "
        f"NON_UPGRADEABLE_CONTRACTS={len(non_upgradeable_list)}, "
        f"total crates covered={len(declared)}/{len(all_crates)}"
    )
    return True


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--fix-hint",
        action="store_true",
        help="On failure, print corrected list values for copy-paste.",
    )
    args = parser.parse_args()
    ok = run_checks(fix_hint=args.fix_hint)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
