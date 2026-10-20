#!/usr/bin/env python3
"""Idea coverage for Phase W.

Every idea in the Web proposal has an ID in docs/proposals/registry.md. This checks that:
  - each ID is unique and its idea text still appears in the source doc it cites;
  - each ID is covered by exactly one CHECKLIST.md item ("Covers: ..."), and nothing covers an
    unknown ID;
  - every Phase W item except the spec items (PW.0.x) covers at least one ID, or says
    "Covers none" because it is pure plumbing (streaming, a review gate).
It prints a summary per item; with --todo it lists the IDs still open.
Exit status is non-zero on any error. Standard library only.
"""

import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "docs/proposals/registry.md"
CHECKLIST = ROOT / "CHECKLIST.md"
SOURCES = {
    "web": ROOT / "docs/proposals/the-web.md",
    "ideas": ROOT / "docs/proposals/the-web-ideas.md",
}
ID = r"[A-Z][A-Z0-9]{1,5}-\d{2}"
ITEM = re.compile(r"^- \[( |x)\] \*\*(PW\.[0-9]+\.[0-9]+[a-z]?)\*\*(.*)$")


def norm(text: str) -> str:
    text = re.sub(r"[*`_]", "", text.lower())
    return re.sub(r"\s+", " ", text)


def expand(token: str) -> list[str]:
    m = re.fullmatch(rf"({ID})\.\.(\d{{2}})", token)
    if not m:
        return [token]
    prefix, first = m.group(1).rsplit("-", 1)
    return [f"{prefix}-{n:02d}" for n in range(int(first), int(m.group(2)) + 1)]


def main() -> int:
    errors = []
    docs = {k: norm(p.read_text(encoding="utf-8")) for k, p in SOURCES.items()}

    registry = {}
    for line in REGISTRY.read_text(encoding="utf-8").splitlines():
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) != 3 or not re.fullmatch(ID, cells[0]):
            continue
        rid, idea, src = cells
        if rid in registry:
            errors.append(f"duplicate ID {rid}")
        registry[rid] = (idea, src)
        doc = src.split()[0] if src else ""
        if doc not in docs:
            errors.append(f"{rid}: unknown source '{src}'")
        elif norm(idea) not in docs[doc]:
            errors.append(f"{rid}: '{idea}' not found in {SOURCES[doc].name}")

    owners = defaultdict(list)
    items = {}
    for line in CHECKLIST.read_text(encoding="utf-8").splitlines():
        m = ITEM.match(line)
        if not m:
            continue
        done, item, rest = m.group(1) == "x", m.group(2), m.group(3)
        covered = []
        if "Covers:" in rest:
            # The list runs to the next emphasis marker (*Needs:*, *Done when:*).
            part = rest.split("Covers:", 1)[1].split("*", 1)[0]
            for token in re.findall(rf"{ID}(?:\.\.\d{{2}})?", part):
                covered.extend(expand(token))
        elif not item.startswith("PW.0.") and "Covers none" not in rest:
            errors.append(f"{item}: Phase W item without a Covers list")
        items[item] = (done, covered)
        for rid in covered:
            owners[rid].append(item)
            if rid not in registry:
                errors.append(f"{item}: covers unknown ID {rid}")

    for rid in registry:
        if not owners[rid]:
            errors.append(f"{rid}: not covered by any checklist item")
        elif len(owners[rid]) > 1:
            errors.append(f"{rid}: covered by several items {owners[rid]}")

    done_ids = sum(len(c) for d, c in items.values() if d)
    print(f"ideas: {len(registry)}  covered: {sum(1 for r in registry if owners[r])}  "
          f"done: {done_ids}  items: {len(items)}")
    if "--todo" in sys.argv:
        for item, (done, covered) in items.items():
            if not done and covered:
                names = ", ".join(f"{r} {registry[r][0]}" for r in covered if r in registry)
                print(f"{item}: {names}")
    for e in errors:
        print(f"error: {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
