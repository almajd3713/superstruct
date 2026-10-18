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

## State at handoff (updated 2026-10-18, end of PW.2.7)

**Done and committed:** PW.0.1–PW.0.7, PW.1.1–PW.1.3, PW.2.1–PW.2.7. `GEN_VERSION` is still 6: nothing is assembled into `l1_cell` yet (the Web L1 is assembled at PW.2.8, which bumps to 7, puts `web` back into the L0 golden digest in `crates/gen/tests/golden.rs` and regenerates the goldens). **Next item: PW.2.8** (arteries, assembles the Web L1, `GEN_VERSION` 7), then PW.2.9…; `python3 scripts/coverage.py --todo` lists it. Also open: PW.2.4b (openings and dressing on Tilted skins) and PW.2.6b (voids in Tilted and Vertical stacks; void mouths cut the skin).

**New modules in `crates/gen/src`** (each has its spec section and tests):
| Module | What | Spec |
|---|---|---|
| `regime_web.rs` | `WebFields` on `Regime`, per-style tables, orientation policies and `frame_at/l1/l2` | `regimes.md` §7, §9 |
| `stack.rs` | the stack of an L0 cell: crusts, Between kinds, face-keyed seams, `u_units` for Flat/Tilted/Vertical/Curved (Curved is Flat until wells, PW.6) | `regimes.md` §8 |
| `l1_between.rs` | Between content STR-03..11 as `WebPiece { tag, prim }`; shares `Gen`, `Run`, `runs`, `begin` with the surface module | `levels.md` §4W.6.1 |
| `l1_surface.rs` | skin plates with openings (HOR-02 as data), roofscape, ceiling-scape, skin, section cliff | `levels.md` §4W.6.2 |
| `l1_void.rs` | voids (VOID-01..13) as keep-out data, whole per L0 cell and clipped per L1 cell; bores by line | `levels.md` §4W.3.3 |
| `l1_fill.rs` | the fill field per L2 cell (FILL-01); `slice --fill` draws it | `levels.md` §4W.4 |
| `l1_frame.rs` | beams on every line, kinds, knots, mega-bridge census | `levels.md` §4W.2.1a |

**Tools:** `slice --stack` (stacks) and `slice --between` (elevation of one L1 layer with Between, surface and frame pieces, one colour per ID; pieces are drawn as projected bounds). Pictures are in `out/` (git-ignored).

**Decisions made while building (change only with the user):**
- Features hang on a lattice site and a run key, never on the cell, so cells agree; lattice pitches divide 4096 so nothing crosses an L1 face sideways; nothing crosses an L0 side face (that is the section cliff).
- A Tilted stack wraps: its last and first Betweens are one run. `tilt_shift` uses the frame's `sqrt` so skin slabs lie on the stack.
- Between density is calibrated to `fill_between / 255` over 2–20 and saturates near 8 % above (a review note caught a first version that saturated at fb 10).
- Beam density is about 3 per cell, below the 4–10 in the spec, because spans are partial; the census (PW.2.11) may retune.
- Stair beams are rare (they need a skew direction at slope 1/2).

**Practical notes:**
- `scripts/check.sh` takes several minutes. `streamer::tests::band_radii_change_live` is timing-flaky and passes alone.
- Face-agreement tests sample two points `2·eps` apart, so they allow about 1 % tangent differences and exclude beam ends, which fall on faces.
- Commit one item per commit, no AI attribution (user rule).
- Older open sign-offs from Phases 1–5 are unchanged; see `docs/handoff-phase5b.md`.
