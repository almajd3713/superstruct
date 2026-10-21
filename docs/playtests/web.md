# Phase W measurements (append-only)

Notes written while the user was remote (2026-10-21). Numbers come from tools; nothing here is a
playtest. The user's 🎮 reviews and the Phase W playtest (PW.12.2) are still to come.

## 2026-10-21, `GEN_VERSION` 29

### Far against near (HOR-03, HOR-06; `crates/gen/tests/far_near.rs`, release, ignored)

Silhouette agreement of the near view (300 m) with the far view (5 km), downsampled to 64 m cells,
over four L1 cells: 99.2, 90.6, 92.0 and 95.3 %. The test asserts at least 85 %. First-surface depth
agreement within 48 m is 35–79 % and is not the criterion (`levels.md` §5W.33).

### Tissue metrics (`web-report --tissue 1500`, estimated per sampled L2 cell)

| Metric | Target | seed 42 | seed 7 | seed 1234 |
|---|---|---|---|---|
| MET-01 built fraction in crusts | 0.70–0.90 | 0.243 | 0.230 | 0.274 |
| the tissue's own aim in the same cells | | 0.815 | 0.806 | 0.809 |
| MET-10 nav nodes with three or more links | ≥ 0.40 | 0.147 | 0.135 | 0.148 |
| MET-08 path types per km of path (per cell) | ≥ 8 | 4.1 | 5.2 | 5.0 |

**Findings.**
- **MET-01 fails on all three seeds.** The tissue fills its cells to its field (0.81, the growth test
  agrees), but the pieces it emits cover about 0.24 of a crust once each structure is judged by its
  envelope boxes and the parent's pieces, on an 8 m grid in 64 m cubes. The envelope boxes of a dense
  cell sum to 0.3–1.4 of its volume with overlaps; the covered union is far lower. Either the
  structures' envelopes are thinner than the slots they own, or the metric's definition is
  stricter than the aim. A decision is needed: tune the density up, or change the target.
- **MET-10 fails.** The nav graph of one cell (`walk::Nav` plates) has a mean of 14 % nodes with three
  links; plates are cut finely and many only touch two neighbours. The definition of a node may be
  too fine; see the same decision.
- **MET-08** is estimated per cell, not per kilometre walked, so it is not comparable with the
  target; the bots (P6.1.1) measure it properly.
- **Not measured:** MET-03 (rays leave the cell), MET-04, 05, 07 (bots on the nav graph), MET-06
  (`PW.3.2` measured the skin at 99.5 %), MET-09 (a test, `every_dead_end_is_paid_off`).

### Defect census (`web-report --defects 150`, seed 42, before the PW.3.15b hubs)

a 21 881; b 1 944 (bridge 792, stair tower 844, walkway 239, ramp 62, other 7); c 18 436 (kind 14
cable 15 977); d 47; e 282; f air 6 103, f rise 6 561, f solid 235; g L2/L2 21 760, L2/L3 639 335,
L3/L3 98 335, stair 2 598; r L2 79. These stay the baselines for PW.4.13 (rises) and PW.4.15 (the
walked pieces of another path that stand in a corridor: bridges, stairs, walkways and ramps).

### Correction, same day: MET-01 counts envelopes

The 0.23–0.27 above counted solid pieces only. A mass that L3 fills is emitted as a hollow shell
(`INTERIOR`), and the spec (`web-report.md` §3) counts L2 envelopes as solid, so `--tissue` now
measures the share of slots that are built, engulfed or solid (every structure by its envelope):

| Metric | Target | seed 42 | seed 7 | seed 1234 |
|---|---|---|---|---|
| MET-01 built fraction in crusts (envelopes) | 0.70–0.90 | 0.609 | 0.658 | 0.651 |
| solid pieces alone | | 0.243 | 0.230 | 0.274 |

MET-01 is close but still under 0.70 on all three seeds; the cells at the edge of a crust, where a
crust fills part of its cell, pull the mean down (the tissue's aim over these cells is 0.81).
MET-10 (0.135–0.148 against 0.40) stands as measured. `PW.12.0` is narrowed to these two.

### Bots on the route network (`cargo run -p bots --release -- --blocks 2`, 8 km block, 3 000 starts, 60 walkers of 3 km)

The bots load every L2 cell of the block, take the tissue's route slots as nodes (edges along the
segments, with their path type) and join the cells through the L1 face ports. Long views come
from 26 rays per node through an occupancy grid of 16 m cells (the tissue's envelopes and the L1
pieces); starts are in the middle 4 km, so the rays stay inside the block.

**A bug found by the bots (`GEN_VERSION` 30).** The first run showed the world falling apart at
every L1 face: the largest component held 13 % of the nodes and no link crossed an L1 face. The
cause: the L1 face ports were given to the L2 cell on the **far** side of the L1 cell (the
`+X` Avenue at `x = 0` instead of `x = 7`), in the artery routing (`l1_artery`) and in the
tissue's anchors. Both now use the cell that touches the face (`p.l2(dir.is_positive())`). After
the fix the largest component holds 86 % (seed 42), 85 % (seed 7) and 79 % (seed 1234) of the
nodes, and about 2 000 links cross L1 faces in the block.

| Metric | Target | seed 42 | seed 7 | seed 1234 |
|---|---|---|---|---|
| MET-04 walking distance to a 2 km sightline (median) | 1.5–2.5 km | 6.2 km | 0.99 km | 1.6 km |
| starts that reach one within 20 km | | 77 % | 86 % | 78 % |
| MET-05 walking distance to breathing space (median) | ≤ 150 m | 8.5 km | 7.0 km | 10.7 km |
| MET-07 vertical share of walked distance | ≥ 0.33 | 0.334 | 0.340 | 0.333 |
| MET-08 path types per km walked | ≥ 8 | 8.5 | 9.0 | 9.3 |
| MET-10 nodes with three or more links (all route slots) | ≥ 0.40 | 0.055 | 0.055 | 0.052 |
| ... of the junctions and ends | | 0.83 | 0.79 | 0.78 |

**Reading.**
- **MET-07 and MET-08 are in range** on all three seeds, narrowly (0.333 against 0.33).
- **MET-04 is in range on two seeds in three**; seed 42's long views are far apart (6.2 km).
- **MET-05 fails by a wide margin.** Only about 180 route nodes in 4 096 cells are near a view,
  set, installation or local place. The payoffs are sparse: one in about 23 cells. Either the
  payoffs are too rare for the metric's definition, or more of the cell's places count (windows
  to a void, cut-aways) than the generator records as payoffs.
- **MET-10** depends on what a node is: counting every route slot, junctions are few; counting
  junctions and ends only, four in five have three or more links, so the web branches.
- Not yet measured: MET-03 (rays inside the cell only), MET-09. PRN-01..08 are the principles
  these numbers stand for; the numbers above are the record for PW.12.1.

### Correction to MET-05, same day

The first bots run counted only payoffs (views, sets, installations, local places), which are
rare, and measured 7–11 km. The spec's definition of near breathing space (`web-report.md` §3)
also counts the near details NEAR-02..05 (slit views, a doorway framing a far light, gaps in the
grating that show the drop, windows), which 4 segments in 10 carry (`Mods::near`). With every
slot of such a segment counted, the median walking distance to breathing space is **0 m (seed
42), 32 m (seed 7) and 32 m (seed 1234)** against at most 150 m: **MET-05 and HOR-11 hold** on all
three seeds. MET-04 stands as above (0.99, 1.6 and 6.2 km against 1.5–2.5 km: seed 7 has more
long views than the range, seed 42 fewer).

### Windows frame times (PW.7.4; native, RTX 4070 Laptop, release, `GEN_VERSION` 30)

`SUPERSTRUCT_AUTOFLY=60` (60 m/s level fly-through) from sector (0, 0, 0) local (450, 560, 200),
5 000 frames, default stream radii (L2 640 m, mid 4 096 m, L1 full 4 096 m, far 51 km),
volumetric fog and the new weather/tools systems on. 51 one-second windows from the game's own log:

- mean 97 fps; steady state 80–140 fps; the first window 0.3 fps (start-up, everything loading);
- the worst frame in a second: median 55 ms, 90th percentile 99 ms, maximum 186 ms (a handful of
  seconds at 48–55 fps with 140–186 ms hitches while chunks stream in);
- the 64 m shell was resident after 4.6 s; at the end 15 L1 full chunks (135 MiB), 59 L2 full
  (509 MiB), 2 082 L2 mid (92 MiB) and 272 L3 (0 MiB) were resident; no "slow frame" line.

No tuning was done and no budget of PLAN §11.4 is claimed. The hitches while flying are the
streaming (chunk meshing and uploads), not the weather or the tools. HOR-15 (how much of the
tissue the occlusion culling hides) was not measured: the renderer has no counter for it yet.

### Occlusion on and off (HOR-15; same fly-through, release, RTX 4070)

| | fps (mean) | scene | prepass | cull, occlusion zones |
|---|---|---|---|---|
| `SUPERSTRUCT_OCCLUSION=0` | 93 | 1.21 ms | 1.08 ms | 0.00 |
| default | 88 | 1.19 ms | 1.07 ms | 0.00 |

The GPU spends about 2.5 ms a frame, so the frame (11 ms) is CPU-bound and the switch changes nothing
that the frame rate or the zone times can show. The share of the tissue that the pass hides is
not measured: PW.7.5 adds a draw counter.

### How much the occlusion culling hides (HOR-15; PW.7.5, `SUPERSTRUCT_CULLSTATS=1`)

Chunks drawn in the last frame (mean of the one-second windows of a 60 m/s fly-through, 19
windows after the first five): with `SUPERSTRUCT_OCCLUSION=0` set A (the frustum) holds **258**
chunks of 864 resident; with the default two-phase pass A holds 246 and B (newly unhidden) 0, so
**247 are drawn**. The frustum removes about 70 % of the resident chunks; the occlusion pass
removes a further **4 %**. HOR-15's expectation that occlusion hides most of the tissue does not
hold at the chunk granularity (an L2 chunk is 512 m and mostly partly visible); the frame rate is
the same (134 and 133 fps) and the frame is CPU-bound (`PW.7.4`).

### More blocks for MET-04 (bots, 2 × 2 × 2 L1, same settings)

| Block | MET-04 median | reach one in 20 km | MET-07 | MET-08 |
|---|---|---|---|---|
| seed 42 at L1 (8, 0, 8) | > 20 km | 35 % | 0.338 | 8.2 |
| seed 42 at L1 (−8, 0, 16) | 4.4 km | 74 % | 0.346 | 9.4 |
| seed 7 at L1 (8, 0, 8) | 5.1 km | 75 % | 0.346 | 8.2 |
| seed 1234 at L1 (−8, 0, −8) | 0.5 km | 94 % | 0.395 | 6.5 |

With the three blocks at the origin (6.2, 0.99 and 1.6 km) the median over seven blocks is
**4.4 km** against 1.5–2.5 km: long views are too rare in typical places (HOR-12 does not hold
on average; the spread between blocks is wide because it follows where the Betweens are).
MET-07 holds everywhere; MET-08 is between 6.5 and 9.4 (target 8). MET-05 (14–32 m) holds
everywhere.

### The principles against the numbers (PRN-01..08; PW.12.1, 2026-10-21, `GEN_VERSION` 33)

| Principle | What measures it | State |
|---|---|---|
| PRN-01 Built is the default; air is carved | MET-01 (crust built fraction, envelopes as solid): 0.61, 0.66, 0.65 on three seeds against 0.70–0.90 | close, short |
| PRN-02 Breathing space shows the extent | MET-05: median 0–32 m to a view detail (target ≤ 150 m); MET-04: median 1.0–6.2 km to a 2 km sightline (target 1.5–2.5 km) in the origin blocks, 4.4 km over seven blocks | breathing space holds; long views too rare |
| PRN-03 Every axis, and every angle, is a direction of travel | MET-07 vertical share of walked distance 0.33–0.40 (target ≥ 0.33); gravity wells and gates (`gravity.md`) | holds |
| PRN-04 Everything crosses everything | connectivity of the route graph across L1 faces: the largest component holds 79–86 % of the nodes of a block (13 % before the port fix of version 30); `routes_connect_every_anchor_in_ten_thousand_cells` | holds since version 30 |
| PRN-05 No dead end without a payoff | MET-09: test `every_dead_end_is_paid_off` | holds (by test) |
| PRN-06 The same grammar at every scale | far-against-near silhouette agreement 90.6–99.2 % (`far_near`); the far lights match the near lit share | holds |
| PRN-07 Variety from combination | MET-08 path types per km walked 6.5–9.4 (target ≥ 8); 617 of 663 registry IDs present in the world or a test | borderline |
| PRN-08 Not every combination is allowed | the legality rules of paths, forms and sets (`paths::legal`, `sets`, `forms`) and their tests, which also filter `GEN_VERSION`-pinned combinations | holds (by test) |

The targets that are not met are listed in `PW.12.0` (MET-01, MET-10), `PW.7.3` (MET-04) and
`PW.4.26` (the defects of the user's walk).

### The census of the world (`web-report --census --size 32768 --l2-cells 12000`, seed 42; 2026-10-21)

The census was L1-only (70 IDs present, 517 absent). It now also reads the features of every L0
and L1 cell of the region, a wide scan of 117 649 L0 cells for sites (wonders, strange regions,
weightless sites, rare events, movers, wells, roads, rivers, spines, ramparts, staircases), the
gate zones round seams, and the L2 tissue of 12 000 sampled cells (forms, supports, variants,
mutations, paths, modifiers, thresholds, hubs, sets, places, quarters, kit, growth, payoffs). IDs
that the world cannot carry (the controller, the tools, the weather, the lights of the engine) are
listed with the test that exercises them. Result: **476 IDs present, 47 tested, 122 with no
instance.** What is still zero, by group:

- **not built**: RIDE-01..10 (the journeys, 🎮), SND-01..09 (sound, audio deferred);
- **measured by the tools, not carried by the world**: MET-01..11, HOR-01, HOR-07..15, PRN-01..08
  (except PRN-05, tested): the report and `docs/playtests/web.md` are their record;
- **no detector yet** (the feature exists): INT-01..19 (the interiors need an L3 sample), ORI-01..16
  (orientation policies), BTW-03, 06, 08, VOID-10, 12, 14, FRM-11, 14, GRV-01, 03, 05..08, 11,
  KIT-13..15, QTR-03..05, LOC-03, SAN-07, 08, SC-05, SC-10, SC-11, RULE-01, 02, TEC-05, 06, TOOL-03,
  LGT-09, WGT-08, WON-13;
- **too rare for the sample**: FORM-24, SV-23, SUP-11 (needs a radial well), STG-11 (beyond 2^20 L0
  cells, the Far Lands).
