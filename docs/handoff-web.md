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

## State at handoff (updated 2026-10-18, end of PW.1)

**Done and committed:** PW.0.1–PW.0.7 (all specs) and PW.1.1–PW.1.3 (oriented primitives). `GEN_VERSION` is still 6: nothing emits oriented pieces yet, and the goldens are unchanged. The game still runs the old generator. **Next item: PW.2.1** (regime fields and orientation policies), then the rest of PW.2 in order; `python3 scripts/coverage.py --todo` lists it.

**Specs written (read before coding the matching item):**

| File | Section | Covers |
|---|---|---|
| `docs/spec/regimes.md` | §7–§12 | Web fields on the regime, strata and Between kinds (64 m units, face-keyed seam Betweens), per-style palettes, stratum character, salts `0x0110–0x0117` |
| `docs/spec/primitives.md` | §2, §9 | oriented shapes, integer frames, helix, clipping, bounds |
| `docs/spec/hashing.md` | §6.1 | no transcendentals in `gen`, rotation tables |
| `docs/spec/levels.md` | §1.4, §3W, §4W, §5W, §6W | L0 line features, L1 line machinery, frame, voids, fill, arteries, L2 tissue framework, L3 multi-entrance |
| `docs/spec/ports.md` | §6W | new connectivity argument (Theorem W) |
| `docs/spec/gravity.md` | all | `g(p)`, wells, gates, lulls, drift, fall rule |
| `docs/spec/traversal.md` | §2.2, §3, §6 | ladders, auto-crouch, fall pointer |
| `docs/spec/web-report.md` | all | metrics, census, bots |

**Key design decisions made while specifying (change only with the user):**
- **The skeleton carries connectivity, not the crusts.** Every face between two live L1 cells has at least 2 ports, every cell has a tree of arteries to its knot, and Betweens are live (catwalks, tethers). Arteries may cross air as spanning types, so the BFS fallback never fails. Theorem W does not depend on stack geometry, voids or the neighbouring regime.
- **Stacks:** crusts and Betweens in 64 m units. The Between at an L0 face is derived from the face key and split half and half, so no crust touches an L0 face along the stack axis.
- **Voids** stay in the inner 6×6×6 L1 cells; Bores are the exception.
- **Oriented pieces** are `Shape::Oriented { frame, origin, local, clip }`. A frame is a key into constant tables (64 headings, 7 tilts, 49 directions); axes use `sqrt` only. Axis-aligned output is bit-identical.
- **Gravity** is a vector sum of well contributions with integer max-norm-8 directions; lulls fall out of cancellation. Gates are L2 zones that override the field locally.
- Provisional numbers waiting for the user's playtest: drift acceleration and decay, 90°/s re-orientation cap, 1.2 m crouch, tilt table steps.

**Code written:**
- `crates/gen/src/orient.rs`: tables, `Frame`, `Oriented`, `helix_stair`, `facet_ring`, `beam_cylinder`, and a test that scans `gen` for banned float functions.
- `crates/gen/src/walk_oriented.rs`: sample-graph nav for oriented pieces. `validate::check_cell` still uses the plate path only.
- Engine: `primmesh::add_oriented` with `clip_polygon`; `FarInstance` is now 64 bytes with an origin and quaternion (far-band buffer 32 → 64 MB, **no timing measured**); WGSL `sdf_oriented_box` in `horizon.wgsl` (matches the CPU to 2e-5 m); collision works through the SDF with no change.

**Still open from PW.1:**
- Screenshots in `out/oriented-*.png` were taken natively on Windows (`scripts/win.sh test -p engine --test render oriented_pieces`). The far band draws oriented solids unclipped (documented in `primitives.md` §9.4).
- The SDF horizon is still the port of the version-6 L1 rules. It gains oriented beams when the Web L1 lands (PW.2.5); it will need the shader port and a CPU-vs-GPU test for each new rule.
- `streamer::tests::band_radii_change_live` failed once under heavy machine load and passes alone; treat it as timing-flaky.
- The `Primitive` size grew (an inline `Oriented` roughly doubles `Shape`). Measure memory and generation time when the first oriented content ships, and box it if it hurts.

**Practical notes:**
- `scripts/check.sh` takes several minutes; run it in the background.
- Render tests skip in WSL. Use `scripts/win.sh test -p engine --test render <name>` and read `out/*.png`.
- Commit one item per commit, no AI attribution (user rule, overrides the harness reminder).
- Older open sign-offs from Phases 1–5 are unchanged; see `docs/handoff-phase5b.md`.
