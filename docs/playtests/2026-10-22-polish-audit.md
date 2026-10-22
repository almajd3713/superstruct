# Polish audit, 2026-10-22 (`GEN_VERSION` 33)

The world still shows too many subtle generation faults: pieces that make no sense as architecture, even where the existing defect census reads low. This note measures them, traces each one to its cause in the code, and proposes fixes. It is meant to be turned into checklist items. Nothing in the generator was changed.

## How this was measured

- **Defect census:** `web-report --seed 0 --defects 200` (200 random L2 cells over the 32 km region).
- **New probe:** a throwaway tool, `out/audit/probe` (git-ignored; built as its own crate on `worldgen`). It counts fault classes the census does not count. It runs over 119 L2 cells: the cells around the 33 viewpoints of the 2026-10-20 and 2026-10-21 walks, plus 14 random 2×2 clusters. It ran on seeds 0 and 42. Its classes:
  - **A:** walk clearance against every solid (L1, final L2, L3);
  - **B:** clusters of solid that touch nothing;
  - **C:** interiors crossed by other solid;
  - **D:** two legs built on one line;
  - **E:** slivers;
  - **F:** walk lines through non-solid decoration;
  - **G:** non-solid plates that look walkable;
  - **H:** what the mid band drops;
  - **L:** L3 pieces outside their mass;
  - **S:** structure parts sliced by the cell's faces.
- **Inspection tools:** `probe inspect <seed> <l2 cell> <x,y,z> <r>` lists every piece near a point. `probe air` finds open air for a camera. `probe views` gives walk-mode poses on legs.
- **Screenshots:** 16 shots on native Windows (`out/audit/*.png`, each with its sidecar). The flat greybox makes distant faults hard to read, so the numbers carry the weight here. The shots are cited only where they show the fault clearly.
- Counts are per 119 cells unless a different base is stated. Seed 0 unless marked 42.

**Replays of old screenshots are not evidence after a `GEN_VERSION` change.** A sidecar keeps the camera, but the world around it is rebuilt by the new version. Replaying the 2026-10-21 shot 0836 (taken on version 29) gave a different scene (`out/audit/r-0836.png`). Several open items (PW.4.26, PW.4.25) count as done when "a replay shows it gone". After any version bump, that check passes whether or not the fault was fixed. Use a census count, or a fresh shot of the same kind of fault, instead.

## Summary, by how much it hurts

| # | Finding | Size | Kind |
|---|---|---|---|
| 1 | Stair walk lines are transformed twice in turned lattices | 99.5 % of turned-rise samples off the stair (seed 42: 97 % of all rises) | bug |
| 2 | Turned lattices are sliced flat at the L2 cell faces | 13.5 % of turned parts (seed 42: 17.5 %); 6–12 % of interiors miss a wall | design |
| 3 | Interiors overlap other solid | 24 % of interiors (seed 42: 35 %) cross another structure; 11–13 % cross L1 | design |
| 4 | Stairs climb into L1 solid and through other paths | 5 900 blocked stair samples, 920 low-ceiling ones | design |
| 5 | Decoration is added after every clean-up pass | dressing with no host back to 19 % (was 0.3 % after PW.4.14) | regression |
| 6 | Pieces float with nothing to hold them | about 1 260 small floating clusters per 119 cells | several |
| 7 | Two paths built on one route | 240 leg pairs, 6 km of doubled walkway | bug |
| 8 | Rails inside enclosed corridors, on unreachable roofs, on both sides of a rim | seen in shots and inspections | design |
| 9 | Unclimbable "stairs" and bare helices among the structure forms | every SV-03, F2-21, SV-16 | design |
| 10 | The mid band (640 m to 4 km) drops every path, bridge and stair | 31 000 large pieces vanish at 640 m | horizon promise |
| 11 | Carve leaves one-quantum slivers | 3 000 (seed 42: 8 900) | cosmetic |
| 12 | Non-solid plates that look like floors | 900 scaffold and ledge plates | trust |
| 13 | The census and its tests miss or miscount several of these | see "Tooling" | tooling |

## Findings

### 1. Stair walk lines are transformed twice (bug)

`paths_v::build_rise` already returns its walk in cell-local metres (`crates/gen/src/paths_v.rs:923`, `m.e.at(q)`). `paths.rs:1899` then maps every point through `e.at` again. In an upright lattice `at` is the identity, so nothing shows. In a turned lattice every rise's walk line is rotated a second time about the cell centre.

- **Measured:** in turned lattices, only 888 of 185 221 stored rise samples have a floor under them. With one `at` undone, 170 247 do (92 %); upright rises reach 97 %. Seed 42 has 16 of 93 931 stored samples on the stair, against 82 640 corrected, and 97 % of its rises are in turned lattices.
- **What reads the walk lines:**
  - `l2_web::ways_of` feeds `edges::cut_rails` and `edges::decor_mask`. In turned lattices, rails are therefore not opened where a stair's foot or top crosses an edge, and decoration is dropped in the wrong places. This is a likely cause of "rails across the join of a landing and a walkway" surviving PW.4.11.
  - The census (`b`, `f rise`) and `web-report --tissue` read them too, so their rise numbers are meaningless in turned strata.
- **Why the tests missed it:** `paths_v::tests::every_type_climbs_the_column` builds in `Frame::Id` only.
- **Fix:** drop the second `e.at` (an output change: rails and decoration move). Add a test that a turned lattice's rise walk has a floor under at least 95 % of its samples.

### 2. Turned lattices are sliced at the cell faces (design)

Each L2 cell's lattice turns about the cell centre. Parts are clipped to the cell box (`Shape::oriented(..., self.cell)`), and pieces wholly outside are dropped (`tissue_grow.rs:1801`, `kit_ext.rs:474`). The neighbour cell has its own lattice and does not continue the part. So along every L2 face of a turned stratum, buildings end in a flat world-axis cut. Hollow shells lose that wall, and their rooms open onto the cut.

- **Measured:**
  - Seed 0: 574 of 4 246 turned parts (13.5 %) reach past a face, with on average 15 % of their volume cut away. No upright part is affected.
  - Seed 42: 1 207 of 6 893 (17.5 %).
  - Interiors with a face that has no shell: 283 of 4 703 (seed 0), 637 of 5 116 (seed 42).
- **Example:** seed 0, L2 (-48,7,-12): a 64×32×32 m mass ends at z = 512, with its shell's +z wall missing. Its neighbour (-48,7,-11) has a 300×32×280 m hollow interior that also starts at the face. `out/audit/s08-slice-seam-out.png` and `s05-slice-seam.png` are taken inside these: a world-aligned wall cuts turned rooms, and floors meet at a diagonal step.
- **Shot `v2-30-8-m10-0.png`:** one straight dark line crosses the floor and the ceiling of a turned skybridge, where it crosses an L2 face.
- **Fix:**
  - **(a)** At placement, reject or shrink any part whose turned box leaves the cell, as far as the clip box. This is cheap and local.
  - **(b)** Turn whole districts about a shared, world-anchored centre so neighbours continue each other. This is bigger but keeps large forms.
  - In both cases, `mass_specs` should refuse a part whose turned box leaves the cell.

### 3. Interiors overlap other solid (design)

`l3_web::mass_specs` (`l3_web.rs:411`) refuses a mass only if it overlaps an earlier mass or a rise's clear box. It does not test:
- solid parts of other structures (a part refused as a mass "stays solid" and fills its neighbour's rooms);
- oriented parts in another frame (FORM-17/18 twisted and leaning towers, turned mutations);
- L1 solid, which a turned lattice crosses between slot centres.

L3 then plans rooms, corridors and doors through all of these.

- **Measured** (share of interior volume, sampled every 3 m):

  | | Seed 0 | Seed 42 |
  |---|---|---|
  | Interiors crossed by another L2 solid | 1 127 of 4 703 (24 %) | 1 809 of 5 116 (35 %) |
  | Interiors crossed by L1 solid | 497 (10.6 %) | 650 (12.7 %) |
  | Interior volume inside other solid | 3.9 % | |

- **Intruders, seed 0** (in samples):

  | Intruder | Samples |
  |---|---|
  | Other structures' envelope parts | 1.28 M |
  | L1 skin | 583 k |
  | L1 tethers | 554 k |
  | Parts in another frame | 337 k |
  | Cages and gratings | 62 k |

- **Example:** seed 0, L2 (-55,11,47) local (80,464,496). A 200×112×150 m turned slab (yaw 8) fills a 96×96×32 m interior of an upright lattice.
- **Effect in play:** corridors end in a blank concrete face, doors open into solid, and rooms are cut in half. The test `every_entrance_reaches_every_other` cannot see this, because it checks the plan, not the geometry.
- **Fix:** test each mass against the final solid it would contain (other parts, oriented parts, L1) before it becomes a mass. Shrink it or keep it solid. Add the C counter to the census.
- **Related:** 1.8 % of L3 solid pieces lie more than 0.6 m outside every mass box: mostly `STAIR_TOWER` (218 k of 15.5 M in 60 cells), then masses (35 k) and misc (18 k). Interior stairs poke out of their building. Example: seed 0, `SECTOR=-28,6,23 LOCAL=573,33,553`.

### 4. Stairs climb into L1 and through other paths (design)

L1 is cut only by the bores of `Bored` segments (`l2_web::l1_primitives_carved`). A rise's column is reserved against structures (PW.4.13), but never against the L1 frame, nor against the decks of the other paths in the cell.

- **Measured** (seed 0; samples at 0.5 and 1.2 m above the walk, which block even the 1.2 m crouch capsule):

  | Blocks | Stair (rise) | Level leg |
  |---|---|---|
  | By L1 | 2 852 | 848 |
  | By another path's pieces | 3 050 | 828 |

  Solid between 1.2 and 1.8 m above a stair, which forces a crouch on a stair: 783 samples by paths, 140 by L1. The worst types are PV-04 (square spiral), PV-01 (straight flight), PV-10 (grand stair), PV-18 (escalator) and PV-12 (stair beam).
- **Examples:**
  - Seed 0, L2 (-40,55,-48): a PV-04 tops out inside a 466×264×466 m L1 block (`SECTOR=-20,27,-24 LOCAL=415,768,194`). This is the "top landing 2 m under a ceiling" of PW.4.26 (h).
  - L2 (-55,12,47): PV-13 and PV-06 flights run through the 0.3 m decks of crossing legs at the same height.
- **Fix:** give a rise a bore through any L1 solid in its column, as bored legs get, and cut that from L1. Reserve the rise column against the other paths when legs are placed, or end the leg at a landing. Count this in the census as walk-clearance against all solid, not only path pieces (census `b` today tests path pieces only).

### 5. Decoration is added after every clean-up pass (regression)

`l2_web::l2_cell_web` runs carve, `orphan_mask`, `fix_members`, `decor_mask` and `cut_rails`, and only then calls `add_extras` (`l2_web.rs:123`, `:145`). `add_extras` adds:
- the exterior kit (`kit_ext::dress`);
- quarter dressing;
- growth;
- strange-region and well geometry;
- gate markers.

None of these are cut by corridors or doors, checked for a host, or kept out of walkways. `kit_ext::dress` dresses every face of every part, buried or not, and keeps only bores clear (`kit_ext.rs:426`).

- **Measured:**
  - Census `c` (dressing with no host): 14 637 of 75 180 dressing pieces (19 %; seed 0, 200 cells), of which 13 616 are cables. PW.4.14 had brought it to 10 of 3 295. The kit, growth and quarters came later (PW.10, `GEN_VERSION` 25 and after).
  - Walk lines pass through decoration (probe F): 914 samples by path misc pieces (lamps, frames), 66 by windows, 45 by quarter dressing.
- **Causes inside the kit:**
  - KIT-07 cable runs stand 0.3 m off the face with no brackets (`kit_ext.rs:296`).
  - KIT-09 lamp chains and GROW-02 roots hang from the undersides of parts, whatever lies under them.
- **Fix:** run the extras before the clean-up passes, or run the passes again on them: host test, corridor and door cut, `decor_mask`. Dress only exposed faces, as `lit_faces` already does.

### 6. Pieces that float (several causes)

The probe's B counter finds clusters of solid pieces that touch nothing: no other L2 piece within 0.15 m, no L1, and at least 1 m from the cell's faces. Contact is tested on bounding boxes, which is generous, so the count is a lower bound.

- **Totals:** seed 0 has 1 259 clusters under 10 m and 816 larger. Seed 42 has 451 under 10 m.
- **The large ones** include whole cell webs of up to 2 900 pieces (seed 42) that touch no L1. They may be legitimately carried by the crust's own tissue. Look at them before calling them faults.

The small ones are faults. By cause:

| What floats | Count (seed 0) | Cause |
|---|---|---|
| PV-04 pillar | in every pillared square spiral | `paths_v.rs:741`: the `TRUNK` slab runs from `y0 + 3` to `top - 3` and stands 1 m inside the flights. It touches nothing. Example: L2 (-55,12,47) local (336,80,464), `out/audit/s04-stair-pillar-far.png` |
| Ceiling-hatch collars | 110 threshold-led | `thresholds.rs:252`: every PosY and NegY entrance gets a 3.4 m collar (`:114`), even in a cell with no routes and with no structure left above it. The edge pass then rails both sides of the 0.5 m rim (9 to 28 rail pieces each). Example: L2 (0,6,4) local (304,96,176), `out/audit/s06-orphan-hatch.png` |
| SV-18 lit windows | 102 | `tissue_variants.rs:596`, `tissue_grow.rs:2382`: panels placed against a host that is not there. They are collidable (`FAR_VISIBLE` only), so `orphan_mask` (which looks at `NO_COLLIDE` dressing) never sees them. Seed 42 has a 229-panel field of them |
| Path pipes and trunks | 274 and 14 | overhead pipes 2 to 3 m above a deck with no hangers |
| Rails and balustrades | 436 | for example, a balustrade standing 0.2 m outside an enclosed skybridge's wall (L2 (-55,12,47) local (128,129,465)) |
| Structure helices and stairs | 519 | see 9 |

**Fix:** a census counter for floating clusters (the probe's B, with a contact test on shapes rather than boxes). Each emitter above gets its host test, or a member to hang from.

### 7. Two paths on one route (bug)

`tissue::route` adds zero to two loop walks between route slots (`tissue.rs:929`). A loop can run along slots already on the tree, and `cut` makes new segments over them, each with its own path type. Both are built.

- **Measured:** 240 leg pairs share a line within 0.6 m over at least 4 m: 6 078 m of doubled walkway among 5 437 legs (seed 0). All pairings occur: PH-01/PH-02, PH-01/PH-04, P2-01/P2-03 and others.
- **Example:** seed 0, L2 (2,6,3) local (240,256,431..465). A PH-04 catenary deck sags under a 1.4 m PH-01 catwalk on the same line. The catwalk has no rails, because the deck 3.5 m below does not count as a drop (`out/audit/v4-2-6-3-0.png`: a railless walkway over a deep fall).
- **Fix:** when cutting segments, skip slot steps already used by an earlier walk.

### 8. Rails where no rail belongs (design)

The edge pass rails any walkable edge above a drop, and a gap of a few centimetres counts as a drop.

- **Inside enclosed corridors:** in P2-01 and PH-02 the walkway is narrower than the walls, and the gap between floor and wall reads as a drop. `out/audit/v61-m40-55-m48-0.png` shows rails along both walls of a closed corridor. Inspecting L2 (-55,12,47) shows a 23 m rail inside a PH-02 skybridge along its wall.
- **On the roofs of skybridges**, which nobody can reach.
- **On both sides of every rim** narrower than a walkway (the hatch collars of 6).
- **Fix:**
  - Treat a gap narrower than the capsule (0.6 m) as no drop.
  - Do not rail a surface that no walk line reaches (a roof, a rim).
  - Build enclosed paths with a floor that meets the walls.

### 9. Structure forms that look like stairs but are not (design)

`Emitter::stair` (`tissue_grow.rs:2049`) fills a whole slot box with a 32-step `StairRun`: treads of 1 m, and risers of 1 to 3 m. It is used for:
- SV-03 "kissing" (a hung structure joined to a tower by a stair: `tissue_variants.rs:262`);
- F2-21 "stair tower field" (`forms2.rs:492`).

These read as giant stairs that cannot be climbed (the step-up is 0.45 m). They are not on any walk, so the player can never use them. The structure helix (SV-16, `tissue_variants.rs:564`, and the rotunda hub) is a 16 to 24 m radius ramp of separate treads up to 112 m high, with no rail, core or landing. The path helix PV-05 got all three in PW.4.13; this one did not.

**Fix:** use a real stair (a `paths_v` flight with landings and rails) where a structure means to join two levels, and put it on a walk. Otherwise make it read as mass, not stair (terraces at least 4 m high, with no tread rhythm). Give the structure helix a core, rails and an exit at the top, or a walk.

### 10. The mid band drops the paths (horizon promise)

`meshpool::mid_primitives` (`crates/engine/src/meshpool.rs:311`) keeps only `MID_ONLY` boxes and envelope parts. Between 640 m and 4 km, the world shows boxes and nothing else: no bridges, walkways, stairs, cages, helices or skin cladding.

- **Measured:** of 108 178 drawn pieces of at least 16 m (seed 0), 31 000 are absent from the mid band. That includes 12 172 walkway pieces, 11 057 bridge pieces, 3 787 stair pieces and 3 253 structure masses without an envelope (cages, inner forms).
- **Effect:** past 640 m the city reads as unconnected boxes (`out/audit/s02-leg-walk.png`: the field below the catwalk), and the web's connecting tissue appears as you approach. That is the change-on-approach CLAUDE.md §9 names as the worst bug.
- **Why the test misses it:** the far-vs-near test compares silhouettes at 64 m and so cannot see it.
- **Fix:** give the mid band a cheap form of the large path pieces (decks and stair runs as boxes, with no rails or detail). Add a mid-vs-full comparison at about 8 m resolution.

### 11. One-quantum slivers from carving (cosmetic)

Doors and corridors are carved out of turned shells in their own frame, and the cuts leave strips 1 quantum (6 cm) thick: 2 227 shell and 821 structure slivers (seed 0), 3 738 and 5 151 (seed 42). They show as thin ribs at door edges (the "stair-stepped cut edges" of PW.4.26 b). A 32 m-tall strip at 1/16 m thick also z-fights at distance.

**Fix:** snap carve cuts to the piece's own quantum grid, and drop or merge any output under 0.1 m in a dimension.

### 12. Plates that look like floors and are not (trust)

The exterior kit's scaffold decks (KIT-12) and ledges are `NO_COLLIDE` gratings and slabs at least 1×1 m: 889 in seed 0. A walker who steps on one falls through. With fall death, that is a trap.

**Fix:** make walkable-looking kit solid (and give it a rail if it is high), or make it read as non-walkable (mesh, not grating).

### 13. Smaller things seen once

- **Diagonal step between rooms:** a floor and ceiling meeting along a diagonal step where a turned room abuts an upright one (`out/audit/s05-slice-seam.png`, likely the face slicing of 2).
- **A gangway that overhangs a junction:** a leg running 0.4 m past the far edge of the path it joins (seed 0, L2 (2,27,20) local (79,448,176)).

## Tooling: what the census and the tests miss

- **Census `g` (coplanar faces) is dominated by a false positive.** Every L2/L3 example pairs an L3 piece with the part's `MID_ONLY` box (flags 18). The full band never draws that box (`meshpool.rs:459`), but the census counts it as drawn (`defects.rs:195`). The 1.69 M `g L2/L3` (seed 0, 200 cells) says nothing about real z-fighting until `MID_ONLY` is excluded.
- **Census `f air` overcounts.** A leg that ends at a T junction, where the crossing path lies behind its end, counts as "over air" (`defects.rs:756` looks ahead and sideways only). The 3 429 of 12 732 (27 %) is an upper bound.
- **Census `b` tests only path pieces as blockers.** Structures, L1, L3, the extras and other paths' decks across a stair are not counted. The probe's A counter tests everything.
- **Census `f rise` and `b` on rises** read the misplaced walk lines of 1 in turned lattices.
- **The census example cap is broken:** the merge caps examples per key, but the keys include the cell prefix (`defects.rs:79`). The seed-0 run printed 881 KB of examples.
- **The walk tests run in upright frames only** (`every_type_climbs_the_column`, `walk_l1`), which hid 1 and parts of 4.
- **Fix:** fold the probe's A, B, C, D, F, G, H, S and L counters into `web-report --defects` (the probe source is in `out/audit/probe/src/main.rs`), fix the four bugs above, and re-baseline. Do this first, because every item below is measured with it.

## Proposed items (to turn into checklist entries)

These are now **PW.13.1 to PW.13.15** in `CHECKLIST.md`, numbered as in the table below; PW.13.0, added the same day, removes the lamps spawned around the player at the user's request.

Suggested order. The first two make the rest measurable; then the bugs; then the design causes, by size. Items marked **(v)** change generator output and need a `GEN_VERSION` bump with new goldens.

| # | Item | Done when |
|---|---|---|
| 1 | Census v2: fix `g` (exclude `MID_ONLY`), `f` (T junctions), example cap; add A, B, C, D, F, G, H, S, L | `--defects` reports them; a baseline for seeds 0 and 42 is written in `web.md` |
| 2 | Proof of a fix is a census count or a fresh shot, never a replay across versions; reword the done-criteria of PW.4.25 and PW.4.26 | the checklist says so |
| 3 | Rise walk lines in the right place (finding 1) **(v)** | the turned-lattice rise test passes at 95 % or more; census on rises re-baselined |
| 4 | One path per route step (finding 7) **(v)** | D = 0 |
| 5 | Parts stay inside their cell, or turn with their district (finding 2) **(v)** | S = 0 in turned strata; no interior with a missing shell face |
| 6 | Interiors only in clear volume (finding 3) **(v)** | C ≤ 1 % of interiors; L3 outside its mass = 0 |
| 7 | Rises own their column through L1 and other paths (finding 4) **(v)** | A blocked on rises = 0; headroom under 1.8 m on stairs = 0 |
| 8 | Extras go through the same passes as everything else; dress only exposed faces (finding 5) **(v)** | census `c` ≤ 0.5 %; F = 0 for decoration |
| 9 | Nothing floats: pillar, hatch collars, SV-18 windows, pipes, balustrades (finding 6) **(v)** | B small = 0 outside weightless and strange regions; the large B clusters reviewed with shots |
| 10 | Rails only on reachable open edges wider than a capsule gap (finding 8) **(v)** | no rail inside an enclosed path or on a roof (new counter); shot of a PH-02 and a P2-01 interior |
| 11 | Structure stairs and helices at human scale, or not stairs (finding 9) **(v)** | no `StairRun` with risers over 0.45 m; every structure helix has a core, rails and a top |
| 12 | The mid band carries the paths (finding 10) | H = 0 for decks and stairs over 16 m; a mid-vs-full test at 8 m |
| 13 | No slivers (finding 11) **(v)** | E = 0 |
| 14 | No fake floors (finding 12) **(v)** | G = 0 |
| 15 | Re-walk: fresh shots of each fault class, then the user's walk | the user signs off |

## Files

- Probe source and outputs: `out/audit/probe/`, `out/audit/probe-w0e.txt` (seed 0, all counters), `out/audit/probe-w42c.txt` (seed 42), `out/audit/defects-s0.txt` (census).
- Shots: `out/audit/s0*.png`, `out/audit/v*.png`, with their `.toml` sidecars. They are listed by name in the findings above.
