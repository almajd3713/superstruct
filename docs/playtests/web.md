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
