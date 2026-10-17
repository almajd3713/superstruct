# Spec: generator levels

What each hierarchy level decides, from what inputs, and with which salts. Coordinates follow `coordinates.md`, geometry follows `primitives.md` and hashing follows `hashing.md`.

## 1. Feature ownership rule

**Decision (P0.2.4, chosen by the user on 2026-10-17): clip to the owning cell, plus column, layer and face keys.** The bounded-overflow option was rejected.

### 1.1 Rule

1. **A cell emits geometry only inside itself.** Every primitive a cell emits lies within `[0, size)` of that cell's local space on every axis. No cell ever places geometry in a neighbour.
2. **A feature may span several cells only if it is derived from a shared key.** When a feature is larger than a cell or crosses a face, every cell it touches derives it from the same key (§1.2) and emits **its own clipped piece**. Only shapes that clip into the same shape may do this:
   - **Box**, clipped on any axis, which gives another box.
   - **Cylinder** and **LiftShaft**, clipped only across faces **perpendicular to their axis**. A vertical trunk can cross Y faces but not X or Z faces.

   Ramps and stair runs never cross a face.
3. **Everything else crosses a face only through ports** (`ports.md`). A bridge from a trunk in one cell to a trunk in the next is two half-bridges, each ending at a shared face port. Both cells derive the port from the face key.
4. **Keep-clear margin.** A keyed feature that must not cross a face keeps its whole footprint at least `margin` inside the cell. For example, an L1 trunk is a vertical cylinder and so may not cross X or Z faces. The margin is a per-level parameter in this spec.

### 1.2 Keys

| Key | Fields | Hashed as | Use |
|---|---|---|---|
| Cell | `level, (x, y, z)` | `hash(seed, level, (x, y, z), salt)` | Anything confined to one cell |
| `FaceKey` | `level`, owner cell, `axis`. The owner is the lower cell on that axis | `hash(seed, level, owner, salt_for_axis)`, with one salt per axis | Ports, and anything shared by exactly two cells |
| `ColumnKey` | `level, x, z` | `hash(seed, level, (x, 0, z), salt)` | Vertical features that run the whole infinite column: the L1 lift core |
| `ColumnSpanKey` | `level, x, z`, span `sy = y div 8` (the parent cell's y) | `hash(seed, level, (x, sy, z), salt)` | Vertical features bounded to one parent-cell height: trunks (L1), shafts (L1), stair towers and lift shafts (L2) |
| `LayerKey` | `level, y`, region `(rx, rz)` with `rx = x div W`, `rz = z div W` | `hash(seed, level, (rx, y, rz), salt)` | Horizontal features: deck heights and thickness over a W×W block of cells |

- Each use has its own salt (`hashing.md` §4). That stops a `ColumnKey (x, z)` from colliding with the cell `(x, 0, z)` or with a `LayerKey` that has the same numbers.
- A key-derived decision must be computed **from the key alone**, never from anything cell-specific, so that every cell sharing the key agrees.
- Cell-specific detail, such as a hole cut into one cell's piece of a deck, is a separate per-cell decision layered on top.
- The region width `W` (in cells) is a per-level parameter, defined with the level's content.

### 1.3 Rationale

- **Locality is O(1).** Evaluating a cell needs only its parent chain and the keys it lies on. It never runs the generator on neighbours. With bounded overflow, every cell would also have to evaluate up to 26 neighbours' features, which multiplies generation cost for streaming in every direction.
- **Agreement is trivial to verify.** Two cells agree exactly when they derive from the same key. That can be tested directly with proptest, unlike checking that "overflowed geometry matches across all neighbours".
- **Clipping is exact.** Clipping a box at an integer-quantum face gives two boxes whose shared face is bit-identical (`primitives.md` §1). There are no seams or gaps.
- **The cost is a placement constraint.** Trunks keep a margin from L1 X and Z faces, so trunk centres never fall in a band near those faces. Decks are continuous across faces through layer keys, which hides the cell grid. The trunk margin is small next to a 4096 m cell, and trunk positions are jittered within the allowed area. The slice tool (P0.10.4) will show whether a grid is still visible. If it is, revisit by making L1 trunk placement come from a coarser key (for example L0-level columns) rather than switching to overflow.

## 2. Conventions for this document

- **Regime inputs.** The L0 fields used below are `density` (packed vs hollow) and `verticality` (pillars vs slabs). Both are `u8` values from 0 to 255, defined in `regimes.md`. Write `d = density / 255` and `v = verticality / 255`.
- **Interpolation.** `lerp(a, b, t)` is `a + (b − a) · t`. It is evaluated in `f32` with the allowed operations only (`hashing.md` §6). Integer weights round toward zero.
- **Units.** Geometry is in metres in this document. It is stored in quanta (`primitives.md` §1), so every length below is a multiple of 1/16 m. Sizes are drawn with `range_i32` in steps given as "step N m".
- **Margins.** Margins are measured from the cell faces, in the owning cell's local space.
- **Status of numbers.** Probabilities and sizes are first guesses. P0.10.4 tunes them with slice images. Changing them after golden tests exist requires a `GEN_VERSION` bump.
- **Salts.** Salts are listed per level (§8). Every decision names its salt in brackets, for example `[TRUNK]`.

## 3. L0: Regime (32768 m)

| | |
|---|---|
| **Inputs** | `seed`, the L0 cell |
| **Outputs** | `Regime` (all fields in `regimes.md`), optional `RareEvent` |
| **Children receive** | the regime and, for a spatial rare event, its region as a `Zone` plus a `KeepOut` where the event removes megaforms |
| **Geometry** | none. L0 only shapes atmosphere and the parameters of the levels below |

- Regime fields are drawn from an RNG seeded with `hash(cell, [REGIME])`. Rare events use `[RARE_EVENT]` (which event, if any) and `[RARE_EVENT_PLACE]` (where in the cell). The events and their probabilities are listed in `regimes.md`.
- **Giant cavity** (the rare event that affects geometry): an irregular blob of **whole L1 cells**, the *cavity cells*, with rounded bites carved from the decks of the surrounding *rim cells* (§4.6). The shape was chosen by the user on 2026-10-17. The coarse shape, computed once per L0 cell from its 8 × 8 × 8 grid of L1 cells:
  1. **Ellipsoid.** Draw with `[RARE_EVENT_PLACE]`:
     - a centre `c` in L1-cell units, uniform in [2.5, 5.5] per axis, step 0.25,
     - radii `r` in [1.0, 3.0] per axis, step 0.25 (4–12 km).

     The draw order is c.x, c.y, c.z, then r.x, r.y, r.z.
  2. **Noisy voxelization.** The L1 cell with L0-local index `i` is a candidate if `Σ ((i + 0.5 − c) / r)² ≤ 1 + n`. The noise `n` is drawn per L1 cell in [−0.25, 0.25], step 1/64 (`range(−16, 16) / 64`, the first draw), from `hash(seed, 1, cell, CAVITY_NOISE)`. The sum is evaluated in x, y, z order. The cell containing `c` is always a candidate. Candidates are restricted to the inner 6 × 6 × 6 (indices 1–6), so there is at least one L1 cell of margin and two cavities never touch.
  3. **Pocket filling.** Flood-fill the non-candidate cells (6-neighbour) from the margin layer. Any non-candidate cell the fill does not reach is enclosed, so it becomes a cavity cell. This guarantees the cavity never encloses an unreachable pocket (`ports.md` Lemma C).

  L1 emits nothing in cavity cells, not even ports. The silhouette reads as an organic abyss at 4 km scale, and the rim bites round its edges up close.
- Regime **values** never blend across L0 faces. Only atmosphere does, in `atmosphere_at` (`regimes.md`). Generation inside a cell uses its own regime only, so L1 cells never need a neighbouring regime.

## 4. L1: Megaforms (4096 m)

| | |
|---|---|
| **Inputs** | the parent regime, the cavity `KeepOut` (if any), column-span, layer and face keys |
| **Outputs** | structural `FAR_VISIBLE` primitives: lift core, trunks, deck pieces and half-bridges. Also L1 face ports (`ports.md`) |
| **Children receive** | `KeepOut` for shafts, chasms, the cavity and bridge clearances. `ForcedPort` for L1 ports that cross L2 faces. A `Zone` for the sanctuary. Connection requirements are given in `ports.md` |

### 4.0 Keyed features and the lift core

L1 is computed in two stages:

1. **Keyed features** are pure functions of keys (and the regime and cavity from L0): the lift core (§4.0.1), trunks (§4.1), decks (§4.2), shafts (§4.3) and chasms (§4.4). They are cheap to evaluate, and port derivation evaluates them for **both** cells next to a face (`ports.md` §3).
2. **Dependent content** reads the keyed features and the cell's ports: deck pieces, bridges, the sanctuary.

**Separation rule.** Within a cell, all hole and obstacle footprints keep **at least 64 m** from each other and from the cell faces. That covers shafts, chasms, the core footprint and the trunk footprint. Placement draws are made in the order core → infinite staircase (if present, `regimes.md` §5.2) → trunk → shaft → chasm, and each later feature is rejected if it would break the separation from the earlier ones. A rejected feature is absent, and the remaining draw counts stay fixed. Rejection is what guarantees each deck stays connected (`ports.md` Lemma A).

#### 4.0.1 Lift core: `ColumnKey(L1, x, z)` `[CORE]`

- **Every** L1 column has one lift core at the same (x, z) in every cell, so the core is an endless vertical spine.
- Footprint: a square, 16–32 m per side, step 4 m (`side = 4 m × range(4, 8)` inclusive). Its min corner is drawn uniformly on a 4 m step, x then z, so that the footprint keeps the 64 m margin.
- It is emitted as a `LiftShaft` with `METAL` box walls. The walls have openings at every stop, and the footprint is a hole in every deck.
- **Stops:** one on every deck top in the cell, and one at every bridge height in the cell (§4.5).
- It passes through every Y face as that face's `LiftStop` port. In cavity cells it is absent.
- **As implemented** (`l1::core_pieces`, P0.7.7): one `LiftShaft` over the full cell height plus 1 m `METAL` walls (kind `SHAFT_WALL`) on all four sides. Each stop opens all four walls from the stop height up to `PORT_CLEARANCE` (4 m). Core pieces are not `FAR_VISIBLE`.

### 4.1 Trunks: `ColumnSpanKey(L1, x, z, sy)` `[TRUNK]`

A span is 8 L1 cells (32 km, one L0 cell tall). Every L1 cell in the span computes the same trunk.

| Parameter | Value |
|---|---|
| Exists | `chance(lerp(0.30, 0.80, v))`, with `v` from the span's L0 cell `(x div 8, sy, z div 8)`. Draw order: exists, radius, centre x, centre z, extent |
| Radius r | 96–400 m, step 16 m |
| Centre (x, z) | uniform in `[r + 64, 4096 − r − 64]`, step 16 m. The 64 m margin applies §1.1 rule 4. If the footprint comes within 64 m of the core or the infinite-staircase footprint, r is reduced in 16 m steps down to 96 m, keeping the centre. If it still conflicts, there is no trunk in this span |
| Vertical extent | bottom cell `a` and top cell `b` within the span, with `0 ≤ a ≤ b ≤ 7`. With probability 0.5 the trunk fills the whole span (a = 0, b = 7). Otherwise a = `range(0, 8)` and b = `range(a, 8)`. The ends lie on L1 cell boundaries |
| Material | `CONCRETE` |

Each cell in `[a, b]` emits the trunk as a vertical `Cylinder` clipped to its own y-range. Trunks in vertically adjacent spans are independent, so stacked segments of different radius form stepped pillars.

### 4.2 Decks: `LayerKey(L1, y, region)` with W = 4 (16 km regions) `[DECK]`

| Parameter | Value |
|---|---|
| Deck count n per layer cell | `pick_weighted([lerp(4, 1, d), 4, lerp(1, 3, d), lerp(0, 2, d)])`, giving 0–3 decks |
| Height of deck i | slot centre `(i + 0.5) · 4096 / n`, jittered by ± a quarter of the slot, step 4 m. Decks are always at least 600 m apart |
| Thickness | 8–48 m, step 4 m |
| Coverage | the whole cell footprint, minus holes (§4.3–4.4) and the core footprint (§4.0.1). Decks are absent in cavity cells |

- **Drawing** (`l1::decks`): `n` first, then per deck from bottom to top: the jitter, then the thickness. The height is the deck's **top** surface, and the slab hangs below it. In quanta, `centre = floor((2i + 1) · 4096 m / 2n)` rounded down to 4 m, and `top = centre + 4 m × range(−J, J)` inclusive, with `J = floor(4096 m / 4n / 4 m)`. The regime is the region's L0 cell, `(rx div 2, y div 8, rz div 2)`.
- All cells in a region and layer agree on heights and thicknesses. Decks therefore continue across every face inside the region, and these crossings are deck-continuation ports.
- At a region face, the deck heights on either side differ in general. The decks end at the face, leaving cliff edges, and are joined only through ports (`ports.md`).
- Each cell emits its deck as `Box` pieces: the cell rectangle minus its holes, decomposed deterministically into rectangles. The algorithm cuts at sorted hole edges and merges cells of the resulting grid row by row. It is pinned by golden tests.

### 4.3 Shafts: `ColumnSpanKey(L1, x, z, sy)` `[SHAFT]`

- Exists with `chance(lerp(0.40, 0.10, d))`, so hollow regimes have more shafts.
- Footprint 64–512 m per side, step 16 m, with width (x) and depth (z) drawn independently. Draw order: exists, width, depth, then the min corner x and z on a 16 m step. Placed under the separation rule (§4.0): it is rejected if it comes within 64 m of the core, the staircase or the span's trunk footprint.
- A shaft cuts every deck in the span (32 km of vertical void). It is emitted as `KeepOut` for children.

### 4.4 Chasms: cell `[CHASM]`

- Exists with `chance(lerp(0.30, 0.05, d))`.
- Footprint 256–2048 m per side, step 64 m, with width and depth drawn independently, then the min corner on a 64 m step. Placed under the separation rule (§4.0): it is rejected near the core, staircase, trunk footprint or shaft. It cuts the decks of this cell only.
- Emitted as `KeepOut` over the full cell height.

### 4.5 Bridges `[BRIDGE]`

- Each half-bridge runs from its BridgeEnd port (`ports.md` §4.1) to a **core stop at the port's floor height**, so bridges are horizontal.
- The route is axis-aligned:
  1. Try going in along the face normal and then turning towards the core.
  2. Otherwise go along the face first and then in.
  3. Otherwise take a 3-segment route through the 64 m band along the faces, which never contains a trunk.
- A route must stay clear of the trunk footprint. If none does, the trunk segment is omitted in this cell (`ports.md` Lemma A.3). Routes may pass over holes, since bridging voids is their point.
- **Trunk landing.** In a cell where the trunk crosses no deck, the first bridge, by port id, gets a landing that touches the trunk. That keeps every trunk reachable.
- **Routes as implemented** (`worldgen::l1_bridge`, P0.7.5). Each route is a chain of boxes at the port's floor, in a frame where `n` runs inward from the port's face and `t` along it. A strip that must meet the core is centred on the core's extent.
  1. **Normal, then turn.** If the port's `t`-range overlaps the core's, one straight box runs to the core's near face. Otherwise one box runs in to a strip aligned with the core, and a second runs along `t` to the core.
  2. **Along, then in.** One box runs along the face within `n ∈ [0, w)` to a strip aligned with the core in `t`, and a second runs in to the core.
  3. **Through a side band.** One box runs along the face to a strip 16 m inside an adjacent face (left side first, then right). A second runs inward along that side band, and a third runs across to the core.

  The first route clear of the trunk (circle distance ≥ r) wins. If none is clear, the trunk piece is omitted and route 1 is used.
- **Trunk landing:** an 8 m wide L from the core to the trunk's axis. It leaves the core along an axis on which the trunk lies wholly beside the core, so it never crosses the core.
- **Children:** each bridge box becomes a `KeepOut` from its underside to 4 m above its floor.
- Width 8–32 m, step 4 m (the port width). Thickness 4 m. Kind `Bridge`, material `METAL`. Only bridges at least 16 m wide are `FAR_VISIBLE`.
- Bridges hang from nothing. At megastructure scale an 8–32 m beam is self-supporting by convention. Supports are dressing, added later.

### 4.6 Giant cavity

- Cavity cells emit no geometry and no ports, and the core is absent there. Faces between a cavity cell and a normal cell carry no ports. Children of cavity cells generate nothing.
- The trunk segments of a column end at the cavity.

**Rim bites `[CAVITY_BITE]`.** A *rim cell* is a non-cavity L1 cell that shares a face with a cavity cell. Each of its deck pieces gets a bite that rounds the cavity's edge:

1. Draw a scale `k ∈ [0.9, 1.3]`, step 1/32, from `hash(cell, CAVITY_BITE)`. The bite region is the horizontal slice, at the deck's height, of the continuous ellipsoid `(c, k · r)` from §3. In world terms this is the same ellipsoid the voxelization used, inflated or deflated slightly.
2. Rasterize the slice on the deck's **64 m tile grid** (64 × 64 tiles per cell). A tile is bitten if its centre is inside the slice. The result is a stepped curve, which is invisible at the scale it is seen from.
3. Unbite every tile within 64 m of a face that carries ports (any face not shared with a cavity cell), of the core, or of the trunk and other holes (the separation rule, §4.0).
4. **Connectivity check:** flood-fill the remaining deck tiles. If they do not form one component touching every port-carrying face band and every core stop, reduce `k` by 1/32 and repeat. If `k` falls below 0.5, there is no bite.

Bitten tiles are removed from the deck pieces before the rectangle decomposition (§4.2). Children receive the bitten area as `KeepOut`.

**As implemented** (`worldgen::l1_rim`, P0.7.8): `k` is drawn as `range(29, 42) / 32`. Tiles are kept (never bitten) on the edge row of every port-carrying face, and within 64 m of any hole (core, shaft, chasm) or of the cell's trunk (radius + 64 m). Connectivity is checked on the 4-connected unbitten tile set, which must be one component containing every kept face-edge tile and every tile within 64 m of the core. A deck whose slice is empty has no bite. `KeepOut` per bite rectangle spans the slab from its underside to 4 m above its top.

The ceiling and floor of a cavity are the deck undersides and tops of the cells above and below. They are not rounded, so vertical rounding comes from the voxel blob alone.

### 4.7 Sanctuary selection `[SANCTUARY]`

- Each L1 cell holds at most one sanctuary, chosen with `chance(0.5)`. That gives about one per two L1 cells, a few km apart.
- The cell lists its L2 children whose volume contains a deck top surface, in x, then y, then z order, and picks one with `below(count)`.
- That child receives `Zone { Sanctuary }`, 32 × 16 × 32 m, placed on the deck top inside the child (§5.5). If no child contains a deck top, there is no sanctuary in the cell.
- **As implemented** (`worldgen::sanctuary`, P0.8.6): draw order is presence, the child pick, then 16 candidate spots on an 8 m step inside a 72 m margin of the child. The first spot where the zone box is fully on deck and clear of L1 content (`Parent::fits`) and under the child's top minus 8 m wins; if none fits, the cell has no sanctuary. L1 emits the `Zone` and sets `MetaFlags::SANCTUARY`; the L2 child holding it copies both, and everything L2 adds (masses, shafts, corridors) treats the zone as a keep-out. Measured over 10⁴ cells: a sanctuary in 30–50 % of cells with a deck and a mean gap of 1.8–3.5 cells along a row (7–14 km).

### 4.8 Far-band reduction (P0.7.6)

`l1_far_primitives(cell)` is the `FAR_VISIBLE` subset: trunks, deck pieces, and bridges at least 16 m wide. It is the same data, not a simplification.

## 5. L2: Districts (512 m)

**L1 context query** (`worldgen::l1_query::L1Context`, P0.8.1): `classify(p)` for a point in L1-cell-local metres returns `TrunkInterior` inside a trunk, else `Structure` inside any other collidable solid (deck, bridge, core wall), else `DeckSurface` for open air within 0.5 m above a deck or bridge top, else `Void`. Lift-shaft volumes and `NO_COLLIDE` pieces are ignored.

| | |
|---|---|
| **Inputs** | parent L1 output (context query P0.8.1: structure, void, deck top, trunk interior), the regime, the parent's `KeepOut`, `ForcedPort` and `Zone` constraints, keys |
| **Outputs** | `ENVELOPE` masses. Structural walkways, ramps, stair runs, stair towers, lift shafts and railings. L2 face ports. Metadata: sanctuary and vista flags |
| **Children receive** | an `Envelope` for each mass (L3 refines them). `ForcedPort` for every port L3 must realize. `Zone` for the sanctuary or vista. `KeepOut` for walkway and port corridors |

### 5.1 Masses `[MASS]`

| Parameter | Value |
|---|---|
| Attempts | n = `pick_weighted` over 0–6 with weights `[lerp(3, 0, d), 2, 2, 2, lerp(1, 3, d), lerp(0, 3, d), lerp(0, 2, d)]` |
| Footprint | 16–128 m per side, step 4 m |
| Height | 16 m to `lerp(64, 256, v)`, step 4 m (4 m floors) |
| Support | on a deck top surface in the cell (bottom = deck top). Trunk-attached masses are deferred (P0.8.9), because they need routing to the network N (`ports.md` Lemma D) |
| Placement | up to 8 tries per mass. The mass must lie inside the cell with an 8 m margin, keep an 8 m gap from other masses and port corridors, not intersect any `KeepOut`, trunk or deck, and be fully supported. If all tries fail, the attempt is dropped |

**As implemented** (`worldgen::l2_mass`, P0.8.2): all draws in order are the attempt count, then per attempt width, depth and height (steps of 4 m), then for each of 8 tries the deck-top pick and the x and z of the min corner (4 m step, 8 m margin), so each attempt draws a fixed amount. Candidate tops are deck tops in the cell with room for 16 m plus the margin. A try is accepted if the box keeps 8 m from the cell faces on top, has 8 m separation on some axis from earlier masses, strictly overlaps no parent `KeepOut` or collidable L1 solid, stays outside the trunk circle, and the footprint is exactly covered by deck pieces at that top. Port corridors join in P0.8.3b.

Masses carry `kind = Mass`, the `ENVELOPE` flag and material `CONCRETE`.
**Envelope fill (L3 obligation):** every face of the envelope must be covered by L3 exterior wall within 0.5 m of the face plane over at least 75 % of its area. All L3 solid stays inside the envelope, except features attached through ports.

### 5.2 Stair towers and lift shafts: `ColumnSpanKey(L2, x, z, sy)` `[STAIR_TOWER]`, `[LIFT_SHAFT]`

The span is the parent L1 cell's 8 L2 cells. A tower connects the walkable levels (deck tops, walkway levels) inside one L1 cell. Movement between L1 cells in Y happens only through L1 Y-face ports.

- **Stair tower:** exists with `chance(lerp(0.15, 0.40, d))`. Footprint 16–32 m. Built from stacked `StairRun`s inside `Box` walls, with a landing at every level it crosses. Those landings are internal lift-stop or landing ports.
- **Lift shaft:** exists with `chance(lerp(0.10, 0.30, v))`. Footprint 8–16 m, emitted as a `LiftShaft`. It has a stop at every walkable level it crosses.
- **As implemented** (`worldgen::l2_column`, P0.8.5, lift shafts only): a shaft's side lengths are `4 m × range(2, 5)` (8–16 m), its min corner is on a 4 m step inside a 72 m margin of the L2 column (clear of the 64 m port corridors), and its rise is `4 m × range(4, 17)`. It stands on **every** deck of the parent L1 cell, with the 12 m ring around it (4 m platform plus 8 m gap) fully on deck and clear of all L1 content (`Parent::fits`), or the shaft is absent. Above each deck it rises to a 4 m wide `GRATING` platform ring at the stop height, with stops at the deck top and the platform, and `lift_pieces` walls open 4 m above each stop. Every L2 cell of the span emits its slice. The shaft and its platform ring are a `KeepOut` for children and for masses.
- The walkway solver (§5.3) also creates towers and shafts when it needs a vertical link. Those are cell-local features, not keyed.

### 5.3 Walkways, ramps and stairs `[WALKWAY]`, `[RAMP]`

**As implemented** (`worldgen::l2`, `l2_walk`, P0.8.4): for each walkway port, each cell builds its half of the skywalk (`Skywalk`): a 0.5 m `GRATING` slab 8 m long from the face at the port's floor, then a `StairRun` (rise 3 Q, tread 5 Q) descending away from the face to the deck top, ending up to 2 Q below the slab (a step). Heights are multiples of 4 m, so every stair is over 3 m and no ramp is needed. The corridor is a `KeepOut`, and masses keep 8 m from it (P0.8.3b). Skywalk halves use only ports and L1 data, so the walkway-port connectivity is by construction; the validator confirms it in tests. Spanning trees between ports, loops and mass entrances are not implemented yet.

This implements cell-level connectivity (CLAUDE.md §3.5, `ports.md`):

1. Gather this cell's ports (cascaded and own face ports), the mass entrances and the tower or shaft landings.
2. Connect them with a spanning tree in a deterministic order (sorted by port id, nearest neighbour first), plus extra links with `chance(0.3)` to create loops.
3. Realize each link as a path:
   - Over an existing deck top, it needs no geometry, only clearance (`KeepOut`).
   - Otherwise it is a `GRATING` walkway box, 4–8 m wide (at least the port width), 0.5 m thick.
   - A height change of 3 m or less uses a ramp of at most 15°. Up to 32 m uses a `StairRun` (rise 0.1875 m, tread 0.3125 m, per `traversal.md`). More than 32 m uses a stair tower or lift shaft.
4. The validator (P0.9) must find every port reachable. A cell that fails is a generator bug (CLAUDE.md §3.5), and the fallback is not to drop the port.

### 5.4 Edges and railings `[EDGE_GUARD]`

- Every walkable edge with a drop greater than the fall threshold (`traversal.md`, about 15 m) is either guarded with `chance(0.85)` or left deliberately unguarded.
- A guarded edge gets a railing: a `Box` 1.1 m tall and 0.1 m thick, kind `Railing`.
- Edges are visited in a deterministic order (by surface, then edge), and the RNG is seeded once per cell.

### 5.5 Sanctuary (from the L1 `Zone`)

- The chosen L2 cell realizes the sanctuary zone on the deck top it lies on: an enclosed room with `WARM` material, at least one port-connected entrance and one opening facing outward.
- The cell's metadata flags it. Interiors and props come in Phase 5.

### 5.6 Vista `[VISTA]`

- An L2 cell with an L1 `KeepOut` void (shaft, chasm or cavity) within 64 m of one of its walkable surfaces gets a vista with `chance(0.25)`.
- **As implemented** (`worldgen::l2_vista`, P0.8.7): draws are the 0.25 chance, a starting side, then eight placement offsets. For each shaft or chasm of the parent and each deck top in the cell, the four sides are tried from the starting side: a 16 m wide strip of deck 8 m behind the void's edge must be fully supported and clear, then the balcony (8 m over the void, a 0.5 m `GRATING` slab flush with the deck) and a corridor (16 m wide, 8 m high, 16–64 m long over the void) must be free of every L1 solid and `KeepOut` except the void's own, of skywalk corridors and lift shafts, and inside the 72 m margin of the cell. The first fit wins; if none does the cell has no vista. The corridor is a `KeepOut`; balcony and corridor form a `Zone { Vista }`; the cell sets `MetaFlags::VISTA`; masses avoid the zone.
- The vista is a walkable platform at the void's edge with a 16 m wide `KeepOut` corridor facing the void, and a `Zone { Vista }` for L3. L3 must keep the corridor open.

## 6. L3: Blocks (64 m), outline

- Rooms, corridors, stairwells, pipe runs and voids are generated with bounded WFC (model synthesis) over the block's 8×8×8 grid of L4 cells.
- Boundary conditions are fixed by `ForcedPort`s, envelopes and zones.
- Failure handling: retry with sub-seeds `[L3_RETRY]`, then fall back to a known-good template.
- Specified in P4.1.1.

## 7. L4: Modules (8 m), outline

- Places code-generated kit pieces (panels, railings, grating, pipes, cables, door frames, lamps) on L3 surfaces and edges `[MODULE]`.
- L5 dressing adds visual-only props inside L4 cells `[DRESSING]`.
- Specified in P4.2.

## 8. Salt registry (initial)

Values follow the blocks in `hashing.md` §4. Port salts are defined in `ports.md`.

| Salt | Value | Key | Use |
|---|---|---|---|
| `REGIME` | `0x0101` | cell (L0) | RNG for regime fields |
| `RARE_EVENT` | `0x0102` | cell (L0) | which rare event, if any |
| `RARE_EVENT_PLACE` | `0x0103` | cell (L0) | event placement in the cell |
| `RARE_STAIR` | `0x0104` | ColumnKey (L0) | infinite staircase (`regimes.md` §5.2) |
| `CAVITY_NOISE` | `0x0105` | cell (L1) | per-cell noise of the cavity voxelization (§3) |
| `TRUNK` | `0x0201` | ColumnSpanKey (L1) | trunk existence, radius, centre, extent |
| `DECK` | `0x0202` | LayerKey (L1, W = 4) | deck count, heights, thickness |
| `SHAFT` | `0x0203` | ColumnSpanKey (L1) | shaft existence and footprint |
| `CHASM` | `0x0204` | cell (L1) | chasm existence and footprint |
| `BRIDGE` | `0x0205` | cell (L1) | bridge width and route choice |
| `SANCTUARY` | `0x0206` | cell (L1) | sanctuary existence and child choice |
| `CORE` | `0x0207` | ColumnKey (L1) | lift core footprint and position |
| `CAVITY_BITE` | `0x0208` | cell (L1) | rim-bite scale (§4.6) |
| `MASS` | `0x0301` | cell (L2) | mass count, sizes, placement |
| `STAIR_TOWER` | `0x0302` | ColumnSpanKey (L2) | keyed stair towers |
| `LIFT_SHAFT` | `0x0303` | ColumnSpanKey (L2) | keyed lift shafts |
| `WALKWAY` | `0x0304` | cell (L2) | extra loop links, walkway widths |
| `RAMP` | `0x0305` | cell (L2) | ramp and stair-run variants |
| `EDGE_GUARD` | `0x0306` | cell (L2) | guarded vs unguarded edges |
| `VISTA` | `0x0307` | cell (L2) | vista existence and side |
| `L3_RETRY` | `0x0401` | cell (L3) | WFC sub-seeds |
| `MODULE` | `0x0501` | cell (L4) | module variants |
| `DRESSING` | `0x0601` | cell (L4) | L5 props |
