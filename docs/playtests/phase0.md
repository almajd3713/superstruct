# Phase 0 exit: connectivity over 10⁶ random cells

Date: 2026-10-17. Generator: `GEN_VERSION` 2. Tool: `slice --report` (`worldgen::validate::check_random`).

## What is checked

For every sampled cell, `check_cell` builds the walkable surfaces of the cell (its L1 parent's window plus everything L2 emitted) and requires that every non-lift port has floor at its opening and that this floor connects, inside the cell, to the L1 network N (Lemma D, `docs/spec/ports.md` §6). It also requires that nothing the cell emits leaves the cell. For every L1 parent in the sample, Lemma A is checked: all deck and bridge surfaces and lift platforms of the L1 cell form one component.

Cells are drawn as runs of the 512 L2 children of one random L1 cell, in a region 65 536 L2 cells wide, at L1 heights −1 to 1.

## Result

```
slice --report --seed 42 --sample-seed 7 --count 1000000 --extent 65536
1000448 cells, 1175075 ports, 0 failures in 3.77s
```

Five more seeds (`1`, `2`, `3`, `777`, `superstruct`, sample seed 11), each over 1 000 448 cells: 0 failures, 1.13–1.16 million ports each, 4.8–5.5 s each (16 threads, WSL2; the time is not a performance figure).

The saved report is `out/phase0-report.json` (git-ignored); its totals are above.

## What the validator caught on the way

Running the check turned up defects that the earlier tests had not:

| Found by | Defect | Fix |
|---|---|---|
| P0.9.2 on L1 cells | Bridges on different faces with floors 4 m apart stacked one slab on the other's floor | Bridge floors on an 8 m grid (`ports.md` §4.1) |
| P0.8.4 on L2 cells | A cascade port derived from a trunk landing that ends on the trunk's axis (buried floor) | Cascade ports inside a trunk are dropped |
| P0.8.4 | A skywalk whose deck top lay below its cell | Own ports need the deck top inside the tile |
| P0.8.8 | Railings on the tops of tall core walls that end in another cell | Railings only on plates inside the cell |
| This run (first 10⁶ pass, 12 failures) | Landing exactly tangent to a trunk's surface gave a port with no floor (9 cells) | One metre of slack in the buried test |
| This run | A vista balcony on a deck whose top is the cell's bottom hung 0.5 m outside the cell (3 cells) | Vista needs the deck top 0.5 m inside the cell |

Those last two changed generator output, so `GEN_VERSION` went from 1 to 2 and the golden file was regenerated.

## Not covered

- Walking itself (controller, step-up, falls) does not exist yet. The validator models the capsule with the rules in `traversal.md` §6.
- The 10⁶ cells sample L1 heights −1 to 1 only.
- L3 and finer levels do not exist yet.
- P0.10.4 (tuning density, variety and void ratio from slice images with the user) is still open and needs a review session.
