# Spec: the `web-report` tool

**Status: PW.0.7, 2026-10-18.** A command-line tool (`tools/web-report`, a new binary crate; `anyhow`, `rayon` for sampling, no GPU) that measures the generated world against the targets of `docs/proposals/the-web.md` §14 and counts every registry ID that exists in it. It is built in PW.2.11. It reads only the public API of `worldgen`; it is headless and runs in WSL.

**Owners:** MET-01..11. The census is the proof of presence for every other item (`CHECKLIST.md`, Phase W "standard done").

```
cargo run -p web-report --release -- \
    --seed 42 --origin 0,0,0 --size 32768 \
    --samples 20000 --bots 64 --bot-km 3 \
    --out out/web-report.json  [--census] [--md out/web-report.md] [--ids SC-,FORM-]
```

## 1. Sample region

- The **sample region** is an axis-aligned box of `size` metres (default 32768 m, one L0 cell) with min corner `origin` (metres from the world origin, `i64`), in a world seed. Default seed 42, origin `(0,0,0)`.
- The region is cut into L2 cells (512 m). Each metric names how it samples. Everything is a function of `(seed, origin, size, samples, bots, bot-km)`: the tool seeds its own RNG with `hash(seed, 0, origin cell, WEB_REPORT)` (salt `0x0F10`, cross-level block) so reports are reproducible and comparable between versions.
- Several regions can be given with `--origin` repeated; the report then lists each one and a pooled summary.
- `--size` smaller than an L1 cell is an error: some metrics need whole L1 cells.
- Streaming is the engine's job, so the tool generates cells on demand with a worker pool (`rayon`, `par_iter` over L1 or L2 cells, the results merged in cell order) and a bounded cache. A 32 km region is 512 L1 cells and 262 144 L2 cells; a full census of L2 and L3 inside it is a heavy run, so L3 is generated only for the **L3 sample**: `--l3-cells` random active L3 blocks (default 2000), chosen by hash.

## 2. Output

A JSON file (schema versioned `web_report: 1`) and, with `--md`, a Markdown table. The JSON has:

```json
{
  "web_report": 1, "gen_version": 7, "seed": 42,
  "regions": [{ "origin": [0,0,0], "size": 32768 }],
  "metrics": {
    "MET-01": { "value": 0.81, "target": [0.70, 0.90], "pass": true, "n": 40213 },
    "...": {}
  },
  "census": { "SC-01": { "count": 5, "cells": 5 }, "...": {} },
  "zero_ids": ["FORM-14", "..."],
  "timing": { "generate_s": 31.2, "bots_s": 8.1 }
}
```

- `pass` is `value` within `target` (inclusive). The tool's exit code is 0 unless `--strict` is given and a metric fails or `zero_ids` is non-empty for the IDs selected by `--ids` (default all).
- Every metric also reports `n`, the number of samples, and where relevant a histogram with 10 bins (`"hist"`).

## 3. Metrics (MET-01..10)

For each metric: the definition, how it is sampled, and the target (the-web.md §14). "Walkable point" means a point on a walkable surface in the finest resident level (`primitives.md` §6), and uses the local `up` (`gravity.md` §6).

| ID | Metric | Definition | Sampling | Target |
|---|---|---|---|---|
| MET-01 | Built fraction in crusts | solid volume (all levels' collidable solid, plus L2 envelopes counted as solid) divided by volume, over points in `Crust(k)` per the stack | `samples` random points in crusts, each judged on a 64 m cube around it: fraction of 8 m cells (L4 grid) that are solid or inside an envelope. The result is the mean of the cubes | 0.70–0.90 |
| MET-02 | Built fraction in Betweens | the same, in `Between(j)` | `samples` points in Betweens | 0.02–0.10 |
| MET-03 | Median sightline from walkable points in crusts | for each sample, 26 rays (the 26 neighbour directions, skipping those that start into solid) cast through the generated geometry (the SDF of `worldgen::sdf` at L1 + L2, no L3) up to 4 km; the sightline is the **median ray length** at the point | `samples/4` walkable points in crusts | 30–150 m |
| MET-04 | Walking distance to a sightline of 2 km or more (median) | from a walkable point, the shortest walking distance along the nav graph to a walkable point with at least 1 of the 26 rays of length ≥ 2 km | `samples/20` walkable points; Dijkstra on the L2/L1 nav graph (bounded at 20 km); points with no such place within 20 km count as 20 km | 1.5–2.5 km |
| MET-05 | Walking distance to near breathing space (median) | shortest walking distance to a point that has an atrium, light well, window to a void or the Between (a payoff of kind view or tagged `LOC-01..05` / `NEAR-*`) | the same sample as MET-04 | ≤ 150 m |
| MET-06 | Crust skin covered by L2 | on the skin of every crust in the region: the fraction of 32 × 32 m tiles that are covered (`levels.md` §5W.7) | all skin tiles in the L1 cells that intersect the region, up to 200 000 tiles | ≥ 0.90 |
| MET-07 | Share of walked distance that is vertical | the sum of the vertical components of path segments (stairs, ramps, ladders, lifts, slopes) over the total path length, for the bot walks | `--bots` bots, each walking `--bot-km` km | ≥ 0.33 |
| MET-08 | Distinct path types per km walked | the number of distinct path-type IDs (PH-*, PV-*, ladder, lift, gate) on the route, divided by the distance in km, averaged over bots | the same walks | ≥ 8 |
| MET-09 | Dead ends without a payoff | nav leaves (route leaves that are not ports, anchors or knots) with none of the four payoffs of `levels.md` §5W.2 step 4 within 2 slots, plus masses with one entrance and no interior payoff (`levels.md` §6W.6) | **all** leaves in the sampled cells | 0 |
| MET-10 | Nav nodes with three or more links | the fraction of nav-graph nodes with degree ≥ 3 | all nodes in the sample cells | ≥ 0.40 |

### 3.1 Bots

The bots are random walkers on the nav graph (the `bots` crate's walker, reused): each starts at a random walkable point, picks a random neighbouring link at each node (with a bias away from the last visited node so it does not go back), and walks until it has covered `--bot-km`. The route is the list of links. Bot stuck cases (no link) and unexpected falls are reported as failures (`"bot_failures"`), as P6.1.1 expects. The bot seed is `hash(seed, 0, origin cell, WEB_REPORT_BOT)` with the bot index absorbed (salt `0x0F11`).

### 3.2 Reporting rules

- The report prints each target and its pass/fail, with the value in the units above (fractions as 0–1; distances in metres).
- A metric that cannot be computed in the region (no crusts, no walkable points) is reported as `null` with a reason, and counts as failed under `--strict`.
- Timing is reported per phase. The tool does not claim performance numbers (CLAUDE.md §7): timing is for tool budgeting only.

## 4. The census (MET-11)

`--census` counts **instances** of every registry ID over the sample region.

- An **instance** is a distinct `(tag, key)` pair (`levels.md` §5W.9). The generator's output carries `Instance` records in `CellMeta`. The tool generates every L1 and L2 cell in the region and the L3 sample, collects the instances, and dedupes by `(tag, key)` so a feature clipped into many cells counts once.
- **Sources of ID tags**, by level: regime and stack (REG-*, ORI-*, STR-*, BTW-*, SC-*), L1 (FRM-*, VOID-*, ART-*, FILL-01, SURF-*, HOR-02), L2 (TIS-*, SUP-*, SV-*, FORM-*, MUT-*, PH-*, PV-*, MOD-*, THR-*, HUB-*, and the payoffs), L3 (L3-*, INT-*), gravity (WELL-*, GRV-*, WGT-*), L0 (RD-*, WON-*), and so on for every group in the registry.
- **Engine-side IDs** (TEC-05..07, CLB-*, GRV-13, GRV-14, …) are not in generator output. For these the registry has a **test name**; the census lists them with `"engine": true` and the name of the test that exercises them (`docs/proposals/registry.md` is extended with a column in a later change; until then the tool reads a small table `tools/web-report/engine-ids.toml`). They are counted as present if the test crate has a test of that name (the tool greps the test names).
- The census output is the map of ID → `{count, cells, first_cell}`. `first_cell` is the L2 or L1 cell where the first instance (lowest hash) occurs, so a screenshot can be taken there with `SUPERSTRUCT_CELL`. The checklist's "seen in an inspected screenshot" step uses this to find a place to look.
- `zero_ids` lists every ID in the registry (minus Deferred) that has no instance in the region. A zero is not an error by itself, because rare IDs (Wonders, strange regions) may need a larger region: `--census --size 131072 --origin ...` or `--find <ID>` which searches outward from the origin, one L0 cell at a time up to 64 cells away, and prints the first cell where it appears.
- `--ids <prefix,…>` restricts the census and the exit code to those prefixes. An item's tick note quotes the output of its own prefixes.

## 5. Interaction with `scripts/`

- `scripts/check.sh` runs `web-report` only in quick mode (`--size 4096 --samples 500 --bots 4 --bot-km 0.5 --l3-cells 100`) once PW.2.11 exists, and fails on a test of the tool itself, not on the targets (targets are tuned over many items and are not a gate until PW.12).
- `scripts/coverage.py --todo` keeps listing the open IDs. A future `--census` cross-check in `coverage.py` could tick "present in the world" mechanically; it is not part of this item.
- The tool lives in the workspace as `tools/web-report`. It depends on `worldgen`, `bots` (for the walker) and `anyhow`; it has no other dependency beyond the pre-approved ones.

## 6. Salts

| Salt | Value | Key | Use |
|---|---|---|---|
| `WEB_REPORT` | `0x0F10` | origin cell (L0) | sampling RNG |
| `WEB_REPORT_BOT` | `0x0F11` | origin cell (L0), bot index absorbed | bot start and choices |

These salts are used only by the tool, never by the generator, so they cannot change generator output.
