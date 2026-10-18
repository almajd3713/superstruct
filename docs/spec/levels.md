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

**Trunk-attached masses** (`worldgen::l2_attach`, P0.8.9, salt `TRUNK_MASS`): after the deck masses, `below(4)` attempts per cell. Each draws a side of the trunk (±x, ±z), outward length and tangential depth `4 m × range(4, 17)`, a height as for masses, then up to 8 tries of a trunk, a deck top, a floor `4 m × range(1, 8)` above that deck and a tangential offset `4 m × range(-8, 9)`. The mass starts 4 m inside the trunk's surface at its chord and extends outward, with its floor in the air. It keeps the 72 m margin, the 8 m gaps and every `KeepOut`. Its outer face gets an 8 m wide `Skywalk` (landing and stair to the deck) whose 64 m corridor must fit like any skywalk's, so the mass is reachable by construction. Corridor and landing are `KeepOut`s for children.

Masses carry `kind = Mass`, the `ENVELOPE` flag and material `CONCRETE`.
**Envelope fill (L3 obligation):** every face of the envelope must be covered by L3 exterior wall within 0.5 m of the face plane over at least 75 % of its area. All L3 solid stays inside the envelope, except features attached through ports.

### 5.2 Stair towers and lift shafts: `ColumnSpanKey(L2, x, z, sy)` `[STAIR_TOWER]`, `[LIFT_SHAFT]`

The span is the parent L1 cell's 8 L2 cells. A tower connects the walkable levels (deck tops, walkway levels) inside one L1 cell. Movement between L1 cells in Y happens only through L1 Y-face ports.

- **Stair tower:** exists with `chance(lerp(0.15, 0.40, d))`. Footprint 16–32 m. Built from stacked `StairRun`s inside `Box` walls, with a landing at every level it crosses. Those landings are internal lift-stop or landing ports.
- **Lift shaft:** exists with `chance(lerp(0.10, 0.30, v))`. Footprint 8–16 m, emitted as a `LiftShaft`. It has a stop at every walkable level it crosses.
- **As implemented** (`worldgen::l2_column`, P0.8.5, lift shafts only): a shaft's side lengths are `4 m × range(2, 5)` (8–16 m), its min corner is on a 4 m step inside a 72 m margin of the L2 column (clear of the 64 m port corridors), and its rise is `4 m × range(4, 17)`. It stands on **every** deck of the parent L1 cell, with the 12 m ring around it (4 m platform plus 8 m gap) fully on deck and clear of all L1 content (`Parent::fits`), or the shaft is absent. Above each deck it rises to a 4 m wide `GRATING` platform ring at the stop height, with stops at the deck top and the platform, and `lift_pieces` walls open 4 m above each stop. Every L2 cell of the span emits its slice. The shaft and its platform ring are a `KeepOut` for children and for masses.
- **Stair towers as implemented** (`l2_column::stair_tower`, P0.8.5b). A 3 Q rise has no whole-step height dividing the 8192 Q cell, so tower flights use a **2 Q rise** and 5 Q tread: 32 steps climb 4 m (64 Q) over 10 m. Deck tops and cells are multiples of 64 Q, so no flight crosses a cell face. Footprint `4 m × range(4, 9)` per side (16–32 m), same margin and clearance rules as lift shafts, and it keeps `2 × (platform + gap)` from the span's lift shaft (the lift is placed first). Flights per deck: `2 × range(1, 8) + 1` (3–15, so the last ends in the east landing). Inside 1 m `STAIR_TOWER` walls (open 4 m at the deck top and at the top): flights alternate between two 4 m lanes, east-going in lane 1 and west-going in lane 2, with a landing across both lanes at the east end after each lane-1 flight. The layout sits against the east wall, so the top landing meets the wall opening, then the platform ring. Landings are not internal ports yet.
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
- **As implemented** (`worldgen::l2_rail`, P0.8.8): flat walkable plates (from the validator's extractor, over the L1 window plus L2 content, masses excluded) are walked edge by edge in -z, +z, -x, +x order, in 2 m cells, and consecutive drop cells form a run of at most 64 m. A cell is a drop if, 0.5 m past the edge at the run's midpoint, no solid fills the 1.8 m above the surface and the nearest solid below is more than 15 m down (or there is none). Each drop run draws `chance(0.85)`; a guarded run gets a 18 Q × 2 Q (1.1 m × 0.1 m) `METAL` box along the inside of the edge. Edges on the cell's faces are skipped, and stairs and ramps get no railings yet.
- Edges are visited in a deterministic order (by surface, then edge), and the RNG is seeded once per cell.
- Edges are probed in 2 m cells and each run of drop cells (up to 64 m) draws once, so a railing never covers an edge stretch that is flush with a landing or doorway (found by the tower test).

### 5.5 Sanctuary (from the L1 `Zone`)

- The chosen L2 cell realizes the sanctuary zone on the deck top it lies on: an enclosed room with `WARM` material, at least one port-connected entrance and one opening facing outward.
- The cell's metadata flags it. Interiors and props come in Phase 5.

### 5.6 Vista `[VISTA]`

- An L2 cell with an L1 `KeepOut` void (shaft, chasm or cavity) within 64 m of one of its walkable surfaces gets a vista with `chance(0.25)`.
- **As implemented** (`worldgen::l2_vista`, P0.8.7): draws are the 0.25 chance, a starting side, then eight placement offsets. For each shaft or chasm of the parent and each deck top in the cell, the four sides are tried from the starting side: a 16 m wide strip of deck 8 m behind the void's edge must be fully supported and clear, then the balcony (8 m over the void, a 0.5 m `GRATING` slab flush with the deck) and a corridor (16 m wide, 8 m high, 16–64 m long over the void) must be free of every L1 solid and `KeepOut` except the void's own, of skywalk corridors and lift shafts, and inside the 72 m margin of the cell. The first fit wins; if none does the cell has no vista. The corridor is a `KeepOut`; balcony and corridor form a `Zone { Vista }`; the cell sets `MetaFlags::VISTA`; masses avoid the zone.
- The vista is a walkable platform at the void's edge with a 16 m wide `KeepOut` corridor facing the void, and a `Zone { Vista }` for L3. L3 must keep the corridor open.

## 6. L3: Blocks (64 m)

**Specified in P4.1.1 (2026-10-18).** L3 turns every L2 mass envelope into architecture: rooms, corridors, stairwells, voids and pipe runs. Each block is solved on its own by model synthesis, inside boundary conditions that its parent fixes completely. Everything outside envelopes is already final at L2.

| | |
|---|---|
| **Inputs** | the parent L2 output (envelopes, ports including entrances, `KeepOut`s, `Zone`s), the regime (`d`, `v`, `style`), block keys and L3 face keys |
| **Outputs** | structural primitives, only inside envelopes, of kinds `Mass`, `StairTower` and `Pipe` (no `ENVELOPE`, no `FAR_VISIBLE`); L3 ports; the slot layout (§6.10) |
| **Children receive** | a `ForcedPort` for every L3 port; `KeepOut`s over door passages, stairwells and entrance passages; copied `Zone`s; the layout, as module slots (§7) |

### 6.1 Active and empty blocks

- A block is **active** if it overlaps (with positive volume) at least one envelope of its parent. Every other block is **empty**: it runs no solve and emits no primitives. Its output is its cascaded ports (§6.6) and the parent's zones clipped to it. That costs one pass over the parent's envelopes, ports and zones.
- **Why empty is a legal answer.** L1 and L2 structure is final (`primitives.md` §5), and every L2 port is already realized by L2 (decks, skywalks, bridges, lift rings). `ports.md` §5 allows finer levels collidable solid only inside envelopes, on their own walkways and as railings. L3 has no walkways outside envelopes, and railings are L4's (§7), so outside envelopes L3 has nothing to add. Pipes, cables and lamps on L2 surfaces are L4/L5 dressing and `NO_COLLIDE`.
- Inside an active block, L3 emits solid **only inside envelopes**. L2 places masses clear of every `KeepOut`, `Zone`, port corridor, lift ring and tower (§5.1, §5.5, §5.6), so L3 solid never meets a `KeepOut`. L3 asserts this in debug builds and the validator checks it (§6.11).
- L3 never re-emits or edits parent geometry. Walkways, stairs, towers, lift shafts and railings stay as L2 made them. The only parent primitive L3 replaces is the envelope, and only while the mass's blocks are resident (§6.12).
- `Zone { Sanctuary }` and `Zone { Vista }` never meet an envelope, so L3 only copies them, clipped, to its children. Building the sanctuary room stays with §5.5.

### 6.2 L2 additions for L3: entrances and links (P4.1.6)

These are **L2 outputs**, computed in `l2`. They are specified here because only L3 gives them meaning, and they change L2 output (§6.13).

**Entrances `[MASS_ENTRANCE]`.** A mass gets at most one entrance: a doorway in its envelope, emitted as a port of kind `Entrance` (`ports.md` §2.1) with a `ForcedPort`. Entrances are numbered after the own ports, in mass order. Every doorway is 4 m wide and runs from the mass bottom to 3 m above it, and it always lies within one IN column (§6.3).

- **Deck masses.** The RNG `hash(cell, MASS_ENTRANCE)` makes two draws per mass, in mass order: `side0 = below(4)` and `j0 = next_u32()`. The sides are tried cyclically from `side0` in the order −z, +x, +z, −x. On each side the candidates are the IN columns of the outermost row facing it, numbered along `u`. Scanning starts at index `j0 mod m` and wraps. A candidate doorway is the middle 4 m of its column (column + [2 m, 6 m)). Its **apron** is the 4 m × 8 m strip in front of it, from the deck top to 4 m above it. The first candidate whose apron is fully on deck at the mass's bottom (which is a deck top), passes `Parent::fits`, and overlaps no L2 collidable primitive and no envelope wins. The apron is emitted as a `KeepOut`. If no candidate fits, the mass has no entrance and L3 seals it (§6.4). Every deck mass makes exactly 2 draws.
- **Trunk-attached masses** (P0.8.9). The entrance is the skywalk's doorway, with no draws. Its `u` range is the lowest 4 m sub-range of the skywalk's 8 m that lies inside one IN column. One always exists: the skywalk is 8 m wide on a 4 m grid, and the mass is at least 16 m deep along `u`. The skywalk landing is the apron.
- Inside a mass every floor sits 0.25 m (4 Q) above its storey line (§6.5). Every entrance therefore has a 4 Q threshold step, which is within the step-up and the same for deck and attached masses.

**Links (§5.3 steps 1–2) `[WALKWAY]`.**
- **Nodes, per deck top in the cell (ascending):** ports whose floor is that top, by port index, anchored 2 m inside the face at the rect's midpoint; deck-mass entrances, by mass index, anchored at the apron's outer midpoint; skywalk stair feet on that deck, by port index; then the lift-shaft and stair-tower openings at that deck.
- **Spanning tree.** Prim's algorithm from node 0. It repeatedly adds the unconnected node at the smallest Manhattan plan distance from the tree. Ties go to the lower new index, then the lower tree index. The tree makes no draws.
- **Extra links (loops).** For each node `i` in order, its nearest other node `j` (same distance and tie rules) is a **candidate**, unless `(i, j)` is a tree edge or already a candidate. Each candidate draws `chance(0.3)` from `hash(cell, WALKWAY)`, in candidate order.
- **Realization.** Each link (tree edge or fired candidate) is a 4 m wide L-shaped clearance path from the deck top to 4 m above it: x-first, else z-first. The first path that is fully on deck at that top, passes `Parent::fits`, and meets no envelope and no L2 collidable primitive is emitted as `KeepOut` boxes. If neither fits, the link is dropped and counted.
- **Why this is enough.** Reachability does not depend on the links: an apron on deck is already in N (`ports.md` §6). The links protect the natural walking lines from any future collidable dressing, and they mark them for L4 (lamp lines, §7). The loop frequency is tested on draws, not on geometry (§6.14).

### 6.3 Slot lattice and slot classes

- **Solve grid.** Each block is 8 × 16 × 8 **slots** of 8 m × 4 m × 8 m (128 × 64 × 128 Q). Columns follow the L4 grid, so every L4 cell holds two slots stacked. The slot index is `i = (t · 8 + z) · 8 + x`, with `t` the storey (0–15, bottom-up). This order is used everywhere.
- **Why 4 m storeys rather than 8 × 8 × 8 cubes.** Every floor height L3 must meet lies on a 4 m grid: deck tops (§4.2), mass bottoms and heights (§5.1), and the floors of skywalks and attached masses. With 8 m storeys, half of all masses would start mid-slot. 4 m storeys give 3.75 m clear rooms: oppressive, but above the 2.4 m minimum. Voids add double and triple heights. In L4-cell terms the solve is still the fixed 8 × 8 × 8 block of P4.1.2.
- **IN columns** of a mass are the world-aligned 8 m columns lying fully inside its footprint. Its **storeys** run from its bottom to its top, `height / 4 m ≥ 4` of them. Each slot falls in one class, decided from the parent alone:

  | Class | Condition | Content |
  |---|---|---|
  | `Out` | in no envelope | nothing |
  | `Rind` | within a mass's height, in a column that meets its footprint but is not IN | solid: the slot ∩ envelope, `CONCRETE`, kind `Mass`, minus any entrance passage (§6.4) |
  | `Core` | an IN column at a storey of the mass | solved (§6.7) |

- The rind is 0–6 m thick on each side; attached-mass faces may fall on a 2 m grid. **Why solid:** it puts the exterior wall flush with the envelope face (the envelope fill of §5.1) without a second facade system, and thick walls suit the style.
- Masses are at least 8 m apart, so a slot lies in at most one mass, and no `Core` slot touches another mass's `Core` slot.

### 6.4 Mass plan and skeleton `[L3_MASS]`

Every block that meets a mass computes the same **plan** for it, from the parent output alone. Masses are numbered in the order of their `Envelope` constraints. Mass `i` uses the RNG `Rng::new(s_i)`, where `s_i` is the `(i + 1)`-th `next_u64()` of `Rng::new(hash(seed, L2, parent, L3_MASS))`. Its draws, in order, are the spine pick, the door direction, then one `below(24)` per storey (step 3).

**Plan class**
- **Sealed:** the mass has no entrance. Every `Core` slot is `Solid`. With the rind, each block emits the envelope clipped to the block as one `CONCRETE` box.
- **Vestibule-only:** exactly one IN column. Its storey-0 slot is a `Vestibule` (a dead-end entrance hall, which is allowed), and every other storey is `Solid`.
- **Full:** two or more IN columns. Steps 1–4 below.

**Full plan**
1. **Entrance column `c_e`:** the IN column behind the doorway. Slot `(c_e, 0)` is a fixed `Vestibule`.
2. **Spine `s`.** The candidates are the IN columns other than `c_e` whose removal leaves the IN set 4-connected. That is every column when both sides of the lattice are at least 2, and the two ends of a 1 × n row. Pick one with `below(n)`. The door direction is picked with `below(k)` among the directions (−z, +x, +z, −x) whose neighbour is IN. Call that neighbour `d`. The spine occupies every storey of column `s`, as fixed spiral tiles with rotation `r` (door towards `d`): `SpiralBase` at storey 0, `Spiral` in between, `SpiralTop` at the top storey.
3. **Storey trees.** Each storey `t` draws `below(24)`, which selects a permutation (in lexicographic order) of the four neighbour directions. The targets at storey `t` are:
   - `d`;
   - `c_e`, at t = 0;
   - for every block column range the mass meets, and every 4-connected component of (IN in that range) ∖ {s}, the component's column nearest `d` (Manhattan distance; ties go to lower z, then lower x).

   A BFS from `d` over IN ∖ {s}, expanding neighbours in the drawn order, gives a tree. The **skeleton slots** at `t` are the union of its paths from `d` to the targets, and those paths' edges are **skeleton edges**. The spine's door edge `(s, d)` is a skeleton edge at every storey.
4. **Entrance passage.** The rind slots between the doorway and `c_e` at storey 0 are solid minus a passage: the doorway's `u` range, from the storey line to 3 m, through the whole rind thickness, with a 4 Q floor slab in it.

The **skeleton** `K` is the spine, the vestibule and the tree slots.

**Why:** connectivity becomes a property of the plan rather than of the solve. Every block part of every storey contains a skeleton slot, so no block ever needs another block's solve to reach the entrance. The plan is a pure function of the parent, so blocks agree without communicating. The spine is the one vertical circulation every Full mass is guaranteed to have. Every other stair is optional variety from the solver.

### 6.5 Tileset

**Sockets.** Horizontal sides take `H = {W, O, D}` (3): wall, open (the room continues), door. Vertical sides take `V = {F, V1, V2, V3, P0, P1, P2, P3}` (8): floor (the upper slot has its own full floor), void of depth k above a floor, and spiral continuing with rotation r. **Adjacency rule:** sockets on a shared side must be equal. That is the only rule, and it is what makes propagation cheap (§6.7).

**Geometry conventions** (slot-local Q; x, z ∈ [0, 128), y ∈ [0, 64)):
- A **floor** is the slot's own slab, [0, 4) over the whole plan, so every walkable floor is at y = 4 Q. The ceiling is the slab of the slot above, which gives 60 Q (3.75 m) of clearance.
- A **roof** is a slab at [60, 64), drawn by every `Core` slot whose upper neighbour is not a `Core` slot of the same mass. It gives 56 Q (3.5 m) of clearance.
- A **wall** is a 4 Q half-wall on the slot's side, drawn by each slot, so back-to-back walls are 0.5 m thick. On rooms it spans y ∈ [4, 64), on voids [0, 64). Walls on z sides span the full side; walls on x sides are inset 4 Q at both ends.
- A **door** is a wall with the opening `u ∈ [48, 80)` (2 m, centred) from the storey line to y = 48. That leaves 44 Q (2.75 m) clear and a lintel at [48, 64).
- Rotations are quarter-turns about the slot's vertical axis. Everything is in whole Q, because 128 is even.

**Tiles.** Sides are listed −z, +x, +z, −x.

| Family | Tiles | Horizontal | Down / up | Geometry | Material, kind |
|---|---|---|---|---|---|
| `Solid` | 1 | W W W W | F / F | the whole slot | `CONCRETE`, `Mass` |
| `Room` | 81 | any of {W, O, D}⁴ | F / F | floor, walls and doors per socket, nothing on O | `CONCRETE`, `Mass` |
| `Atrium` (a room under a void) | 16 | {W, O}⁴ | F / V1 | as `Room` | as `Room` |
| `Void1`, `Void2` | 32 each | {W, O}⁴ | Vk / {F, Vk+1} | walls only, no floor | `CONCRETE`, `Mass` |
| `Void3` | 16 | {W, O}⁴ | V3 / F | walls only | `CONCRETE`, `Mass` |
| `Corridor` | 15 | D at the mouths, W elsewhere: straight 2, bend 4, tee 4, cross 1, dead end 4 | F / F | floor; solid everywhere except a passage 32 Q wide (`[48, 80)` across) and y ∈ [4, 48), from the centre to each mouth | `CONCRETE` solid, `GRATING` floor, `Mass` |
| `SpiralBase` | 4 (r) | D at side r, W elsewhere | F / Pr | full floor plus the spiral | §6.5.1 |
| `Spiral` | 4 | as `SpiralBase` | Pr / Pr | landing c0 and the lane-0 flat as its floor, plus the spiral | §6.5.1 |
| `SpiralTop` | 4 | as `SpiralBase` | Pr / F | c0 and the lane-0 flat, pillar and walls; the rest is open down the well | §6.5.1 |

There are 205 tiles in all, so a domain fits in four `u64` words. Outside the domain are the fixed `Vestibule` (a room whose outer side carries the doorway or passage and whose inner sides are D towards every `Core` neighbour) and the `Rind` and `Out` slots. Ramps are not used: a 15° ramp needs 15 m of run per storey, which does not fit a slot.

#### 6.5.1 The spiral stair

The stair is a square spiral round a solid 4 m pillar `[32, 96)²`. It climbs one storey per slot, so stacked spirals repeat identically.

- **Layout.** The four corner landings are 28 Q squares inside the 4 Q walls: c0 at (−x, −z), c1 at (+x, −z), c2 at (+x, +z) and c3 at (−x, +z). The four lanes are 28 Q wide. Lane j runs along one side from cj to the next corner: lane 0 along −z towards +x, lane 1 along +x, lane 2 along +z, lane 3 along −x back to c0. Each lane starts with a flat at its corner's height and ends with a flight.
- **Heights.** The landings are at 4, 20, 36 and 52 Q, each a 4 Q slab.
- **Lanes 0–2.** A 39 Q flat, then a `StairRun` of 5 steps with a 3 Q rise and 5 Q tread (15 Q over 25 Q), then a 1 Q step onto the next corner.
- **Lane 3.** A 44 Q flat, then a `StairRun` of 4 steps (12 Q over 20 Q) that ends at y = 64, then a 4 Q step onto the next slot's c0 at 68.

Every rise and tread follows `traversal.md` §6, every final step is within the step-up, and no flight crosses the slot top.
- **Headroom** under the next storey's copy: 49 Q (3.06 m) over every flight and 60 Q over every flat and corner, both above the 2.4 m minimum.
- **Door.** On the −z side (rotation 0), it opens onto lane 0, which gives 23 Q (1.44 m) of flat floor plus the first step, wider than the 1.2 m minimum door.
- **Materials.** Pillar, walls and slabs are `CONCRETE`; landings and flats are `GRATING`; flights are `METAL`. The kind is `StairTower`.
- `SpiralBase` also has a full floor. The nook under lane 3's flat has 44 Q of headroom and is reached from c0. The nooks under lanes 1 and 2 are too low to walk, and the walkability extractor drops them.

**Weights** (per tile; first guesses, tuned in P4.1.3). These are integers: computed in `f32` per §2, rounded toward zero, with a minimum of 1. Side factors are f(W) = 2, f(O) = 3, f(D) = 2.

| Family | Weight per tile |
|---|---|
| `Solid` | `lerp(300, 3000, d)` |
| `Room` | Π f(side), family total 2401 |
| `Atrium` | Π f(side) × `lerp(0.1, 0.6, v)` |
| `Void1–3` | Π f(side) × `lerp(0.6, 0.1, d)` ÷ (number of up variants) |
| `Corridor` | `lerp(20, 160, d)` |
| `Spiral*` (solver-placed) | `lerp(10, 60, v)` |

**Style multipliers:** Pipeworks ×2 on `Corridor` (and on pipe runs, §6.9); Hollow Expanse ×2 on `Void`/`Atrium` and ×½ on `Solid`; Pillar Forest ×2 on `Spiral*`; the others ×1. Pipeworks also uses `METAL` for room half-walls. Material choice makes no draws.

**Proved properties of the tileset** (each is an exhaustive unit test over the 205 tiles and every matching pair):
- Every walkable tile has one floor region that touches each of its non-W sides at y = 4. For the spirals, the walkable surface is c0 plus the lane-0 flat, with the other storey reached through the spiral.
- Every floor region has at least 2.4 m of headroom, except the declared spiral nooks.
- Every step is ≤ 7 Q and every tread ≥ 4 Q.
- No piece leaves its slot.
- A void column is at most three slots deep, so every drop inside a mass is ≤ 12 m, below the 15 m fall threshold, and needs no railing (`traversal.md` §6).

**Left to L4/L5:** wall panels, door frames, railings, grating detail, lamps, cables, window panels on facades, pipe brackets and bends.

### 6.6 Boundary conditions and face agreement

Before solving, the sockets of every `Core` slot's sides are fixed or restricted as follows:

| Side faces | Socket |
|---|---|
| `Rind` or `Out` (the exterior), or the mass bottom or top | W horizontally, F vertically. The roof is drawn by the geometry rule in §6.5 |
| a fixed slot (spine, vestibule) | that slot's socket |
| a skeleton edge inside the block | restricted to {O, D}; both slots are restricted to walkable tiles |
| a `Core` slot of the same mass across a **block X/Z face** | fixed from the face pattern (below) |
| a `Core` slot across a **block Y face** | P_r on the spine column, F elsewhere. Voids and solver spirals therefore never cross block Y faces |

**L3 face key.** A block face is `FaceKey { level: L3, owner, axis }` (`ports.md` §1). Its draws use `hash(seed, L3, owner, PORT_FACE_X | PORT_FACE_Z)`, the cross-level port salts (`ports.md` §7): no new salt is needed. A face pattern exists only on faces inside an L2 cell, because masses keep 8 m from L2 faces. Both blocks therefore share one parent, and so the same masses, plans and regime.

**Face pattern** (X/Z faces). There are 16 × 8 slot pairs, iterated by storey `v`, then column `u`. Each pair draws `chance(lerp(0.04, 0.10, 1 − d))`, all 128 always, so the draw count is fixed. The socket of the pair is:
- **D** if the pair is a skeleton edge;
- otherwise **D** if both slots are free `Core` slots of the same mass and the draw fired;
- otherwise **W**.

Both blocks compute exactly the same pattern, so they agree without communicating (CLAUDE.md §3.4). **Why** D, never O, across faces: a room never spans two blocks, so each block's air spaces close within the block, and the sealing rule (§6.8) stays local.

**L3 ports.**
- **Cascade.** L2 ports on L2 faces that coincide with this block's faces, clipped to the block's face, with the same kind; then the `Entrance` ports whose doorway lies in this block (converted to block-local space).
- **Own ports.** Every D on an X/Z block face becomes a `Walkway` port: `u` is the column's offset + [48, 80), `v0` is the storey line + 4 Q (the floor on both sides), and `v1` is the storey line + 48 Q. Every spine crossing of a Y face becomes a `StairLanding` port over the 128 × 128 Q column.
- **Numbering:** cascade, then entrances, then own ports by face (−X, +X, −Y, +Y, −Z, +Z), each by (`v`, `u`).
- **Margin:** L3 own ports keep 3 m from column edges, so they never touch a face edge (`ports.md` §2).
- Crossings of L2 internal structure over block faces (deck tops, skywalk slabs) are **not** listed. They are structural, already final, and nothing finer realizes them.

### 6.7 Solver `[L3_SOLVE]`, `[L3_RETRY]`

Model synthesis over the block's free `Core` slots, with the boundary conditions of §6.6 fixed. There is no backtracking.

1. **Initial domains.** Every free `Core` slot starts with all 205 tiles, restricted by its fixed sides, its skeleton restrictions and its storey (no up-P in the top storey of the mass, no down-V in storey 0). Fixed slots are pre-collapsed and act only through their sockets.
2. **Propagation.** AC-3 with a FIFO queue, seeded in slot order. For a slot and a direction, the **present sockets** are the sockets on that side among its remaining tiles: at most 3 or 8 values, each tested with a precomputed 256-bit mask per (direction, socket). The neighbour's domain is ANDed with the union of the masks for the opposite side of the present sockets. A changed neighbour is enqueued if it is not already queued. An empty domain is a **contradiction**, and the attempt fails.
3. **Selection.** Collapse the uncollapsed slot with the fewest remaining tiles (popcount), with ties going to the lowest slot index. **Why this rule:** fewest-remaining avoids most contradictions, and the index tie-break keeps it deterministic.
4. **Choice.** `pick_weighted` over the remaining tiles' weights (§6.5), in tile-id order: exactly one draw per collapse. Then propagate.
5. **Acceptance:** §6.8.

**RNG.**
- Attempt 0 uses `Rng::new(hash(seed, L3, block, L3_SOLVE))`.
- Attempt `k` (1 ≤ k < K) uses `Rng::new(q_k)`, where `q_k` is the k-th `next_u64()` of `Rng::new(hash(seed, L3, block, L3_RETRY))`.
- The draws within an attempt are only the per-collapse picks, in collapse order. Pipe draws are separate (§6.9).

**Retries: K = 6, then the fallback.**
- Under independent attempts, 99.9 % success needs a per-attempt failure rate `q` with `q⁶ ≤ 10⁻³`, so `q ≤ 0.32`.
- Socket equality plus `Room`'s full {W, O, D}⁴ alphabet means a horizontal contradiction cannot arise unless vertical sockets (spirals, voids) pin a slot's sides. The expected `q` is therefore small (a few per cent), and K = 6 leaves a wide margin for "hard" blocks whose failures are correlated.
- Six attempts at about 0.5 ms, plus the fallback, stay within the 5 ms budget (P4.1.5).
- If P4.1.3 measures a fallback rate above 0.1 %, the tile weights and face-opening rate are tuned, not K.

**Cost bound.** A block has at most 1024 slots, domains of four words, and at most 8 sockets per direction. One propagation step therefore costs at most 2 × 8 × 4 word operations, and selection is a linear scan of 1024 popcounts per collapse, which is about 1 M simple operations per attempt. All buffers are fixed-size arrays reused per worker, with no allocation inside the loop. Typical attempts are expected to take 0.2–0.5 ms in release.

### 6.8 Acceptance, sealing and the fallback

After an attempt collapses every slot, build two slot graphs over the block's `Core` slots:
- the **walk graph**: O or D sides between walkable slots, and P sides between stacked spirals;
- the **air graph**: every non-W, non-F side (O, D, Vk, Pr).

A walkable slot is any `Room`, `Atrium`, `Corridor`, spiral or `Vestibule`.

1. **Seal.** Every air component that contains no skeleton slot and no face D becomes `Solid`. It is closed: only W and F separate it from everything else, so nothing outside can see into it.
2. **Collapse dead voids.** Process the void slots bottom-up. A void whose lower neighbour is now `Solid` becomes `Solid`, unless it has an O side to a walkable slot of the walk component of `K`. In that case the solid top becomes its floor, one 4 Q step below the rooms beside it. This prevents pits that can be fallen into but not left.
3. **Accept** if every remaining walkable slot is walk-connected, inside the block, to a skeleton slot. Otherwise the attempt fails. The only way to fail is a face door whose room cannot reach the skeleton.

**Fallback (always solvable, no RNG).**
- **Rooms.** Let R be the free `Core` slots that are:
  - skeleton slots;
  - on a **fallback path**: a BFS at one storey, inside the block and the same IN component, avoiding the spine, from each face-D slot to the nearest skeleton slot;
  - or adjacent to a fixed slot's D side.

  Each slot in R is a `Room` (up F) with D on every side whose neighbour is in R or is a fixed D, unless that side is fixed W. Every other side is W.
- **Everything else** free is `Solid`.

**Lemma L3-1 (the fallback is legal for every boundary condition).** Sockets agree on every side:
- **R–R:** D on both sides, except where fixed W, and then W on both.
- **R–Solid:** W–W.
- **R–fixed:** the fixed socket. The spine's only D faces `d`, which is a skeleton slot at every storey and so in R. The vestibule's D sides face `Core` neighbours, all in R. Rind/Out sides are W.
- **Face pattern:** a face D has both of its slots in R (they are fallback-path roots); a W stays W.
- **Vertical:** every Room and Solid has F/F; the spine is a fixed P stack; block Y faces are F off the spine.

Every fallback path exists, because step 3 of §6.4 puts a target in every IN component of every block part at every storey. Every tile used exists: `Room` with sides in {W, D}, and `Solid`. The construction uses only the boundary conditions, so it exists for every combination of entrance, spine, skeleton and face pattern the parent can produce.

**Lemma L3-2 (skeleton connectivity).** In a Full mass, `K` is connected in the walk graph across blocks, and it contains the vestibule:
- each storey's tree is connected (a BFS tree over IN ∖ {s});
- tree edges are O/D inside a block and fixed D across block faces, in both blocks;
- `d` meets the spine's door at every storey;
- the spine's P stack joins all storeys;
- `c_e` is a target at storey 0.

**Lemma L3-3 (block closure).** In every accepted layout, solved or fallback:
- every walkable slot is walk-connected inside its block to `K` (acceptance rule 3; for the fallback, by its paths);
- every face D is joined to `K` on both sides;
- every other air space is sealed.

**Lemma L3-4 (geometry realizes the slot graph).** Equal floors at y = 4 in every storey, with the tile properties of §6.5, turn every walk-graph edge into a link of the walkability `Nav` (`primitives.md` §6). Pipes keep at least 2.75 m clear (§6.9), so they remove no link.

**Theorem L3 (L3 attaches to N).** Every L3 port, every entrance and every unsealed L3 walkable surface connects to N **inside the parent L2 cell**. The chain is:
1. Lemmas 3 and 4 join any of them to `K`.
2. Lemma 2 joins `K` to the vestibule.
3. The vestibule opens on the entrance.
4. The entrance's apron is deck, which is N (§6.2), or the attached mass's skywalk, which L2 connects to N (§5.1).

This is CLAUDE.md §3.5 at L3. "Inside that cell" is read as **inside the parent L2 cell**: a block buried in a mass contains no part of N, so the per-block form is Lemma 3 ("reaches the mass skeleton"). The validator checks the theorem directly (§6.11).

### 6.9 Pipe runs `[L3_PIPE]`

- **Pass.** Pipes are an overlay, added after acceptance, so they never affect solvability. The RNG is `hash(seed, L3, block, L3_PIPE)`. Draws, in order:
  - the run count `n = below(N + 1)`, with `N = lerp(1, 6, d)`, doubled in Pipeworks;
  - then per run: the storey `below(16)`, the axis `below(2)`, the line `below(8)`, the pipe count `range_i32(1, 4)`, and three offsets `below(5)`. All three offsets are always drawn.
- **Route.** A run follows the longest stretch of consecutive `Room`, `Atrium` and `Void` slots on that line and storey that contains the line's first such slot. It never crosses the block boundary, a `Rind` slot, or any other tile. If there is no such slot, the run is empty, and its draws still count.
- **Geometry.**
  - Each pipe is a horizontal `Cylinder` of radius 4 Q along the run, with its axis at y = 52 Q above the storey line. That occupies y ∈ [48, 56), level with the underside of the door lintels, and leaves 44 Q (2.75 m) clear above the floor.
  - Pipes are placed across the slot at 16 Q + 24 Q × offset.
  - Material `PIPE`, kind `Pipe`, collidable.
  - Pipes pass through half-walls; overlapping solid is allowed (`primitives.md` §3, union).

### 6.10 Output

`l3_block(seed, l2_cache, block) -> (CellOutput, L3Layout)`. Both are pure functions of their arguments, and the golden digest covers both.

- **`primitives`**, in this order:
  1. `Rind` and `Solid` slots merged into maximal boxes: runs along x, then equal runs along z, then along t. The merge is pinned by golden tests.
  2. Every other slot's tile pieces, in slot order, each tile's pieces in a fixed order.
  3. The pipes.

  Sealed masses are one box per block.
- **`ports`:** as in §6.6.
- **`child_constraints`**, in this order:
  1. a `ForcedPort` for every port;
  2. `KeepOut` over every door passage (the opening plus 1 m on each side, floor to lintel);
  3. `KeepOut` over every spiral slot's interior minus the pillar;
  4. `KeepOut` over every entrance passage and vestibule doorway;
  5. the parent's `Zone`s clipped to the block.
- **`meta`:** `flags` (copied SANCTUARY or VISTA if a zone meets the block), plus a new `L3Meta { attempt: u8, fallback: bool, sealed_slots: u16 }`. `attempt` is 0 to K − 1, and K when the fallback was used.
- **`L3Layout`:** per slot, the class, the mass index, the tile id and the sealed flag; plus the pipe runs. It is the hand-off to L4 (§7) and to the slice tool.
- **Budget.** At most 4096 primitives per block, checked by a test.

### 6.11 Validator (P4.1.4)

`check_l3(seed, cache, l2_cell)` works per L2 cell, because Theorem L3 is stated inside the parent L2 cell.

**Building the Nav:**
1. the L1 window (as in `check_cell`);
2. the L2 output with the `ENVELOPE` primitives removed;
3. every active block's L3 primitives, shifted into L2-local space by the exact offset `(b mod 8) × 1024 Q`.

Plates of L1 origin are N, as before.

**Required:**
1. Every non-lift L2 port still has floor and reaches N. L3 must not break L2.
2. Every entrance has floor outside (apron deck or skywalk landing) and inside (vestibule), and both reach N.
3. Every L3 own port has floor on both sides of its opening (`plates_at_port`), and both reach N.
4. **No stranded floor:** every plate of L3 origin reaches N, except plates on an envelope's top (roofs). This also catches pits and unreachable rooms, which a ports-only check would miss.
5. No L3 primitive leaves its block. No collidable L3 solid lies outside an envelope or meets a parent `KeepOut`.
6. **Envelope fill:** every envelope face is covered by L3 solid within 0.5 m of its plane over at least 75 % of its area (§5.1).

Headroom and steps are enforced through the Nav (plates need 29 Q of headroom, links ≤ 7 Q), and the 2.4 m design clearance is proved per tile (§6.5). `check_random_l3` samples L2 cells in a fixed order on several threads, like `check_random`, until the active blocks reach the target count.

### 6.12 Streaming and rendering hooks

- **Chunks.** The streamer requests an L3 chunk only for **active** blocks (listed cheaply from the L2 output, §6.1) within `l3_radius_m = 300 m` of the camera, the near band of PLAN.md §4. Distance is measured to the block's AABB. A 32 m hysteresis applies. Priority sits below the L2 near chunks.
- **Swap per mass.** A mass's L3 blocks are drawn, and collided with, only once **all** of its blocks are resident: at most 3 × 5 × 3 = 45. The L2 chunk then stops drawing that mass's envelope. Until then, the envelope is drawn and collided with. This follows the "finest resident level" rule (`primitives.md` §5) and avoids a half-built mass.
- **Far never simplifies fine.** The mid band draws L2 envelopes, the far band draws L1 data. No L3 mesh is ever decimated into a farther band. Consistency between the envelope and its L3 facade comes from the envelope fill (§5.1), which is checked by P2.7.1 and P6.1.3.
- **Expectations,** to be measured in P4.1.5 and P4.2.3:
  - **Per active block:** 300–1500 primitives (cap 4096) and 10–40 k triangles after internal-face removal, with a worst case of about 120 k. Output is about 30–100 KB.
  - **Active blocks within 300 m:** about 20–150, depending on density. That is roughly 0.5–5 M triangles in dense regimes, which **risks** the 6 ms geometry budget. L4 instancing and the measurement in P4.2.3 decide it, and any miss is reported (CLAUDE.md §4).
  - **Generation:** at under 5 ms per block on 8 workers, about 1600 blocks/s, enough for a 100 m/s tram crossing the 300 m band in 3 s.

### 6.13 GEN_VERSION and golden tests

- L3 is a pure consumer of L2. Building L3 (P4.1.2–P4.1.5) changes no L2 output, so the L2 golden digests stay valid. Until P4.1.6, L3 is reachable only through `l3_block` (tests, tools, bots), not through the engine's generator entry. The entrance function (§6.2) is written first and called directly by L3.
- **One bump, 3 → 4,** in the commit that lands P4.1.6. That commit makes L2 emit entrances, aprons and links (an L2 output change) and enables L3 in the generator entry (new output for a level that had none). Both changes are deliberate, so the L2 golden digests are regenerated in that commit.
- 100 L3 golden blocks (active blocks only, picked deterministically, like the existing per-level lists) are added to `tests/golden/cells.txt` when P4.1.3 lands. Their digests cover `CellOutput` and `L3Layout`. They do not change at the bump, because `GEN_VERSION` is not hashed.

### 6.14 Test plan

| Item | Test | Size | Pass |
|---|---|---|---|
| P4.1.1 | Spec review against this table | — | user sign-off |
| P4.1.2 | Toy tilesets: a 2-tile checkerboard on 4 × 1 × 4; a 3-tile pipe set; a forced contradiction; fixed boundaries respected | exhaustive | exact layouts; a contradiction returns `Err`, never panics |
| P4.1.2 | Exhaustive tile proofs (§6.5): floor regions, sides, headroom ≥ 2.4 m, steps, slot bounds; and every matching tile pair links in `Nav` | 205 tiles, all matching pairs | all pass |
| P4.1.2 | Pinned solve: one 8 × 16 × 8 block at a fixed seed | 1 | layout digest equal to its pin (draw order) |
| P4.1.3 | Fallback rate over active blocks sampled across 8 seeds | 10⁵ blocks | fallback ≤ 0.1 %; histogram of attempts recorded |
| P4.1.3 | Forced fallback (attempts = 0), then validate the L2 cells | 10⁴ blocks | 0 failures in `check_l3` |
| P4.1.4 | `check_random_l3` | L2 cells covering ≥ 10⁴ active blocks | 0 failures of checks 1–6 |
| P4.1.4 | Face agreement: for every pair of adjacent active blocks in the sample, both compute the same ports and the same 128 sockets for the shared face | ≥ 10⁴ faces | identical |
| P4.1.4 | Determinism: blocks generated single-threaded in slot order vs 8 threads in shuffled order, with fresh vs shared L2 caches | 4096 blocks | identical digests |
| P4.1.5 | Bench `cargo bench -p gen --bench l3`, native Windows release | 1000 busiest blocks plus 1000 empty | p99 < 5 ms busy; empty < 20 µs; recorded in PLAN.md §11.4 |
| P4.1.6 | Entrances: masses without one | 10⁴ L2 cells | ≤ 1 %, reported |
| P4.1.6 | Loop frequency: fired / candidates | ≥ 10⁴ candidates | within 0.30 ± 0.015 (about 3σ) |
| P4.1.6 | Loops exist: realized extra links | same cells | ≥ 1 in at least 20 % of cells with ≥ 3 nodes |
| P4.1.6 | `check_l3` after the change | ≥ 10⁴ active blocks | every port and entrance reachable; 0 failures |

## 7. L4: Modules (8 m), outline

- Places code-generated kit pieces (panels, railings, grating, pipes, cables, door frames, lamps) on L3 surfaces and edges `[MODULE]`.
- **Input from L3 (fixed by P4.1.1).** An L4 cell holds two slots of its parent block's `L3Layout` (§6.10). L4 reads the layout through an L3 cache, as L2 reads L1, and derives its module slots from it rather than from parsing primitives:

  | Module slot | Taken from |
  |---|---|
  | Wall faces | the inner face of every W side of a walkable slot. Facade faces (the outer faces of the rind and of exterior W sides) take `NO_COLLIDE` window and emissive panels |
  | Floor cells | every walkable slot's floor at y = 4 Q. Corridors and spiral flats take grating overlays |
  | Door openings | every D side (door frames). L3 ports and entrances are `ForcedPort`s and stay open |
  | Edges | `Room` O sides facing a void (optional atrium balustrades; every drop is ≤ 12 m, §6.5). Railing modules replace the look of L2's P0.8.8 railing boxes in place. No edge inside a mass needs a guard |
  | Lamp sockets | the ceiling centre of every `Room`, `Atrium`, `Corridor` and `Vestibule` slot, every spiral corner, and the deck link paths of §6.2 |
  | Pipe routes | the runs of §6.9 (brackets, and bends where a run meets a wall), plus `NO_COLLIDE` cable bundles |

- **Collision rule for L4.** Collidable additions only within 0.25 m of an L3 wall face, or at least 2.65 m above a storey line, and never inside an L3 `KeepOut`. Everything else is `NO_COLLIDE`. This keeps every walk link of Lemma L3-4 intact without re-validating.
- L5 dressing adds visual-only props inside L4 cells `[DRESSING]`.
- Specified in P4.2.

## 8. Salt registry (initial)

Values follow the blocks in `hashing.md` §4. Port salts are defined in `ports.md`. L3 face patterns (§6.6) reuse `PORT_FACE_X` and `PORT_FACE_Z` at level L3. `L3_MASS` belongs to the L3 block although it is keyed by the L2 parent, because every block of a mass must derive the same plan.

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
| `TRUNK_MASS` | `0x0308` | cell (L2) | trunk-attached masses (§5.1, P0.8.9) |
| `MASS_ENTRANCE` | `0x0309` | cell (L2) | deck-mass entrance side and column (§6.2) |
| `L3_RETRY` | `0x0401` | cell (L3) | sub-seeds of solve attempts 1 to K − 1 (§6.7) |
| `L3_SOLVE` | `0x0402` | cell (L3) | solve attempt 0: one `pick_weighted` per collapse (§6.7) |
| `L3_MASS` | `0x0403` | cell (L2), the parent | mass plans: per-mass sub-seeds, spine, door direction, storey tree orders (§6.4) |
| `L3_PIPE` | `0x0404` | cell (L3) | pipe runs (§6.9) |
| `MODULE` | `0x0501` | cell (L4) | module variants |
| `DRESSING` | `0x0601` | cell (L4) | L5 props |
