# Handoff: Phase W, the Web (2026-10-18)

Read `CLAUDE.md`, then this file, then the Phase W section of `CHECKLIST.md`. This phase rebuilds the generator around the design in `docs/proposals/`. It is large: 112 items covering 662 ideas. This file is how to work through it without losing any of them.

## Why

After playing the Phase 5 build, the user found the world empty and dull: deck slabs kilometres apart with sparse boxes on them, and buildings that were dead ends. They chose a redesign, recorded in PLAN.md §10.4:
- dense strata separated by Betweens;
- an infinite web of routes on every axis;
- free orientation, ladders and gravity wells;
- rare long views, where far landmarks always lead somewhere new.

## The documents

| File | What it is |
|---|---|
| `docs/proposals/the-web.md` | Architecture: levels, strata, frame, arteries, tissue, orientation, gravity, climbing, horizon promise, metrics |
| `docs/proposals/the-web-ideas.md` | The idea bank: stratum character, Wonders, Roads, quarters, forms, paths, interiors, payoffs, light, weather, presence, tools, strange regions, sanctuaries, sound |
| `docs/proposals/registry.md` | Every idea with a stable ID, and the rules |
| `CHECKLIST.md` Phase W | Every item lists the IDs it owns (`Covers:`) |
| `scripts/coverage.py` | Fails if an ID is unowned, owned twice, or no longer matches its doc. Run by `scripts/check.sh`. `--todo` lists the open items with their ideas |

## How to do an item

1. Run `python3 scripts/coverage.py --todo` and take the first open item, respecting *Needs*.
2. For each covered ID, read the doc section the registry cites. Treat the text as a family to explore, not a single shape: give it parameters and variants (the user insists on this; see the note on creativity below).
3. Write or extend the spec in `docs/spec/` for every covered ID before coding.
4. Implement it. Tag the primitives, zones and modules it produces with the ID, so the census can count them.
5. Prove each ID:
   - the census (`web-report --census`, once PW.2.11 exists) counts it in the sample region;
   - or, for engine-side ideas, a test exercises it.
6. Look at a screenshot of each ID. Name the shots in the tick note.
7. Tick the item with the date and a note, then commit one item per commit. No AI attribution.

**Until the census exists** (before PW.2.11), prove presence with a unit test that generates a cell and finds the tagged output.

## Rules that protect the whole design

- Do not drop, merge or reword a registry row to make the script pass. If an idea is impossible or bad, ask the user. Their answer goes into the Deferred table.
- If an idea grows during implementation (new variants worth keeping), add rows with new IDs and put them on the current item or a new one.
- Invariants in CLAUDE.md §3 still hold. With free orientation, primitives carry clip boxes so they stay inside their owning cell (the-web.md §7.3). Rotations come from constant tables, never from `sin` or `cos` in `gen`.
- Any output change bumps `GEN_VERSION` and updates the goldens deliberately.

## Note on creativity

The user wants wild, varied ideas, not literal readings. Their examples, and the ones in these docs, are seeds. When implementing a family, multiply its variants. When stuck for ideas, research or ask the user.

## State at handoff

- Nothing of Phase W is built. `GEN_VERSION` is 6, and the game still runs the old generator.
- Next item: **PW.0.1** (specs before code).
- Older open sign-offs from Phases 1–5 are unchanged; see `docs/handoff-phase5b.md`.
