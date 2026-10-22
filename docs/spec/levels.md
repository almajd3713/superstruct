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

### 1.4 Oriented pieces (PW.0.3)

Rule 2's list of shapes that may cross a face is replaced from version 7. **Any** shape, at any angle, may cross a face if it is derived from a shared key, because each cell emits its own piece **clipped to the cell box** (`primitives.md` §9.4). The two pieces of one solid meet on the face plane exactly, since both clips use the same integer plane. The features of §4W (beams, voids, spines, roads) are all of this kind. Ramps and stair runs with a rise still stay inside one cell unless they are part of such a keyed feature.

## 2. Conventions for this document

- **Regime inputs.** The L0 fields used below are `density` (packed vs hollow) and `verticality` (pillars vs slabs). Both are `u8` values from 0 to 255, defined in `regimes.md`. Write `d = density / 255` and `v = verticality / 255`.
- **Interpolation.** `lerp(a, b, t)` is `a + (b − a) · t`. It is evaluated in `f32` with the allowed operations only (`hashing.md` §6). Integer weights round toward zero.
- **Units.** Geometry is in metres in this document. It is stored in quanta (`primitives.md` §1), so every length below is a multiple of 1/16 m. Sizes are drawn with `range_i32` in steps given as "step N m".
- **Margins.** Margins are measured from the cell faces, in the owning cell's local space.
- **Status of numbers.** Probabilities and sizes are first guesses. P0.10.4 tunes them with slice images. Changing them after golden tests exist requires a `GEN_VERSION` bump.
- **Salts.** Salts are listed per level (§8). Every decision names its salt in brackets, for example `[TRUNK]`.

> **Web redesign (PW.0.3, 2026-10-18).** From `GEN_VERSION` 7 the content of L1 is replaced by §3W–§4W below (stack, frame, voids, fill field, arteries). Sections 3 and 4 stay as the description of versions 1–6 until PW.2.8 removes the code they describe. Everything in §1 (ownership and keys) still holds, with the oriented clipping of §1.4.

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

## 3W. L0 in the Web: stack, line features, wells

*Version 7. The L0 cell still draws a regime (`regimes.md` §1, §7). It adds the following.*

### 3W.1 Stack

The regime's **stack** (`regimes.md` §8) is a pure function of the L0 cell, with seam Betweens derived from the L0 face keys. L1 cells ask it three questions, all answered in integers:

- `stratum_at(p)` → `Crust(k)` or `Between(j)` for the stack coordinate `u(p)` of the point, in units of 64 m;
- `crust_extent(k)` → the `u` interval of crust `k`;
- `character_at(p)` → the `Character` of `regimes.md` §10, `None` in a Between.

For `Flat` stacks `u` is the cell-local `y` in units; for `Tilted`, `Vertical`, `Curved` the integer formulas of `regimes.md` §8.1 apply.

### 3W.2 L0 line features

Great Roads, Spines, Bores, Rivers, the Rail Abyss and the Cable Highway (`the-web-ideas.md` §3, registry `RD-*`) are keyed by **L0 lines**. They use the line machinery of §4W.1 with the L0 lattice pitch (§4W.1.4). In L1 they are **forced arteries** (§4W.5.5): a line forces a port on every L1 face it crosses, and the cell connects its entry and exit ports by an artery of the line's type. What each line is made of is specified by the items that own it (PW.10.x); the rules that make them safe for connectivity are in §4W.5.5.

- **Spine.** `ColumnKey(L0, x, z)` `[SPINE]`: exactly one Spine in every L0 column (the user's "about one every 30 km"). Its L1 column is `(below(6) + 1, below(6) + 1)`, which keeps it inside the inner 6×6 of the L0 cell, and its L2 column in that L1 cell is `(below(6) + 1, below(6) + 1)` (draws in the order L1 x, L1 z, L2 x, L2 z). It runs through **every** L0 cell of the column and every L1 cell of it, from the bottom of the world to the top. A Spine has a lift (`lifts.md`), a stair that never ends and a station on every crust and Between it crosses.
- **Voids may not touch the L0 faces** (§4W.3), so line features that cross an L0 face never meet a void there.

### 3W.3 Wells

Gravity wells are specified in `gravity.md` (PW.0.6). L0 only holds their centres: an integer point per well from the L0 key `[WELL]`, about one per 50 km. They matter here in two places: `StackGeometry::Curved` (`regimes.md` §8.1) and the radial orientation policy.

### 3W.5 Rare events (PW.9.1–9.3, `worldgen::rare`, `GEN_VERSION` 22)

One L0 cell in six holds one rare event (`RARE_SITE`, `0x0124`, keyed by the L0 cell): the kind is uniform over the fourteen of RARE-01..14 and the site is an **inner** L1 cell (indices 1–6 on each axis, so a district stays in its L0 cell). `rare_at(seed, l1)` is the event that touches an L1 cell: its own site, or a district that reaches it. The draws of geometry come from `RARE_BUILD` (`0x0125`), keyed by the site's L1 cell. Every event is on top of the older rare events of `regimes.md` §5.1.

| ID | Kind | Reach | What it does |
|---|---|---|---|
| RARE-01 | Inverted district | the site's L1 cell and its 26 neighbours | support weights are zero except Hanging (×8) and Suspended; mutation 1 (inverted) always applies |
| RARE-02 | Repeating corridor | the site's cell | one corridor of 6 × 4 m and 4 km along x through the middle of the cell, a frame (two posts and a lit beam) every 8 m, identical in every section; the cell keeps the space clear |
| RARE-03 | Scale break | district | the stratum's scale is the largest of the table, ×4 (not ×8: the table's end) |
| RARE-04 | Dollhouse | district | the scale is ×½ |
| RARE-05 | Hollow trunk | the site's cell | 16 facets of 32 m (inner radius 80 m) the whole height of the cell, and a helix of 64 treads a turn (0.4375 m rise, 4 m wide, inner radius 48 m) inside, 146 turns |
| RARE-06 | Builder site | the site's cell | four posts of 290–500 m, beams every 96 m (each present half the time), a scaffold of poles and grating decks up one face, a crane on a mast |
| RARE-07 | The web | the site's cell | a void of 1 km on a side crossed by ten catwalks (3 m wide, with rails) at 88 m intervals and fourteen vertical cables; the catwalks reach 16 m past the void |
| RARE-08 | Lamp sun | the site's cell | an emissive cube of 160 m with four lamps of 48 m, far-visible |
| RARE-09 | Tram graveyard | the site's cell | 48 tram bodies (24 × 3.2 × 3.2 m) at random places, headings and tilts in a 760 m square, 40 m deep void |
| RARE-10 | Planetoid | the L0 cell | the L0 cell's wells are replaced by one Point well at the site (reach 9 km, strength 256); the stack is `Curved` round it whatever the regime drew, so the crusts wrap; the cube core of `gravity.md` §3.4 is 2 km |
| RARE-11 | Möbius corridor | the L0 cell | a Fold well that is a square loop of 1.2 km (reach 300 m): upright on two sides, upside down on the other two. **Deviation:** the field is single-valued, so a lap must cross an even number of flips; the loop is upside down for half of it instead of returning you inverted |
| RARE-12 | Shattered grain | district | every box of a form is turned by a frame of its own (`mutations::shatter`), 5°–30° of tilt, any heading |
| RARE-13 | Section cliff | the site's cell | a wall 4 km tall and 2 km wide, a thousand floors every 4 m cantilevered 128 m out of it, nine columns; the 700 m in front is kept clear |
| RARE-14 | Quiet lull | the L0 cell | a hollow cube chamber (half size 128 m, 16 m walls) with four rails of 4 cm across it and a lamp; two equal twin wells at ±200 m make its middle a lull; the twins leave no planetoid and no furnace (`Well::quiet`) |

The geometry events cut the L1 pieces out of the space they keep clear (boxes are cut, other shapes are dropped if they really enter it), list it as a `KeepOut` for the children and tie the L2 cells round it to the artery tree (`TieKind::Rare`), so the routes of the tissue reach it. The events with no geometry of their own (the districts, the wells) add none (`rare_content` is empty).

### 3W.6 Roads (PW.10.1, `worldgen::roads`, `GEN_VERSION` 23)

**Gathering.** `worldgen::features` collects what the world-scale features add to an L1 cell (`rare`, `roads`, `spines`, `rivers`): pieces in cell-local quanta and the space each keeps clear. `l1_web` cuts that space out of the cell's other pieces (boxes are cut, other shapes dropped if they really enter it, `enters`), lists it as a `KeepOut` and adds the pieces; the artery skeleton ties the L2 cells round it to the tree.

A road is a function of an **L0 line**: the row `(y, z)` of L0 cells for a road along x, the row `(y, x)` for one along z (`ROAD_LINE`, `0x0126`, keyed by the row and the axis). A line holds a road with chance 1 in 10; the road then runs the whole row at one inner L1 row `(a, b)` of each L0 cell, one deck height (1024–3072 m in the cell, on a 128 m step) and one width, and the deck centre is the cell centre, so every L1 cell on the line derives the same road and the deck meets itself at every face. Kinds, by weights `[4, 2, 2, 2, 1, 1]`: **RD-02 Lantern** (a lamp post every 50 m on both edges, lit), **RD-03 Dark** (the posts unlit), **RD-04 Pilgrim** (12 m wide, a shrine arch with a candle every 256 m), **RD-05 Freight** (24 m wide, rails inlaid at ±5 m, a gantry every 512 m), **RD-07 Rail Abyss** (a trestle with piers of 160 m every 128 m, two tracks of two rails, a catenary mast every 64 m; no cars) and **RD-08 Cable Highway** (pylons of 60 m every 512 m on both sides, a cable on each, a parked cabin every 400 m; the gondola motion is a world-clock mover, PW.10.12). All have a deck of 1 m with rails, piers of 64 m every 256 m, and RD-06: lamp posts every 50 m, a rest niche every 2 km (two a cell) and a way-house of 32 m with a lamp at the door every third cell (about every 12 km: the 10 km of RD-06 is rounded to the cell grid; it is a hall, not a sanctuary zone yet). RD-01 is the whole: a lit processional route you can follow. Test `a_bot_follows_a_road_across_three_l0_cells`: the decks of 24 cells meet end to end at the same height and width and form one run of plates in `walk::Nav`.

### 3W.7 Spines (PW.10.2, `worldgen::spines`)

An L0 column holds a spine with chance 1 in 8 (`SPINE_COLUMN`, `0x0127`) at an inner L1 column of its own; the spine runs through every L1 cell of that column, the same in each: **SPN-01** a tower of 48 m with walls of 2 m (and a far-band column of 32 m); **SPN-02** a lift shaft of 12 m in its middle (`Shape::LiftShaft` over the cell's height; the cars belong to the lift system, which still plans from the v6 core and does not read these shafts); **SPN-03** a round spiral stair of 64 treads a turn between the shaft and the wall, 146 turns a cell, which never ends; **SPN-04** a doorway of 4 × 3.5 m in the wall and a landing of 8 m at every crust surface (the L2 row whose stratum is a crust over a Between, read from the L0 stack at the tower's axis).

### 3W.8 Rivers (PW.10.3, `worldgen::rivers`)

A river is a function of an L0 line like a road (`RIVER_LINE`, `0x0128`), with chance 1 in 14: a bundle of 300–800 m of pipes (three layers of 14 m, radii 3, 4 or 6 m, three in four places filled) at one height (800–2300 m) and one inner L1 row. Each pipe is keyed by the line and its place in the bundle, so the cells of the row draw the same pipes. **RIV-01** plain; **RIV-02** the hot river: warm pipes and bands of light every 64 m; **RIV-03** the dead river: concrete-coloured pipes that break once in two cells for 24–64 m; **RIV-04** the broken river: absent from the middle two cells of every run of eight. A service catwalk with rails runs along the top on posts every 64 m. The bundle's volume is kept clear.

### 3W.9 Wonders (PW.10.8–PW.10.11, `worldgen::wonders`)

An L0 cell holds a wonder with chance 1 in 12 (`WONDER_SITE`, `0x0129`) at an inner L1 cell; its geometry draws from `WONDER_BUILD` (`0x012A`). Every wonder is a feature (`features.rs`): its volume is cut out of the rest of the cell, kept clear of the children and tied to the artery tree, and its large pieces are `FAR_VISIBLE` (32 m or more in every direction). Metres in the cell, centre `c = 2048`:

| ID | Wonder | Built as |
|---|---|---|
| WON-01 | Cenotaph | a hollow cube of 600 m with walls of 24 m, one door of 24 × 60 m, a sarcophagus and one candle |
| WON-02 | Tatlin's Spiral | two leaning helices of 48 girders each (70 × 36 × 36 m, table frames of 30° tilt), 18 m of rise a step, on a radius of 220 m |
| WON-03 | The Dam | 13 of 32 facets of a ring of 800 m, 60 m thick, 400 m tall |
| WON-04 | The Walking City | a platform of 700 m on six legs (24 m radius) with 24 towers; the walking is a world-clock mover (PW.10.12) |
| WON-05 | Tower of Babel, inverted | twelve stepped boxes hung from the cell's top, widest at the top |
| WON-06 | The Organ | forty pipes (6–14 m radius, 150–500 m tall) in five ranks on a platform with a console; the sound is PW.11 |
| WON-07 | The Lamp Sun | an emissive cube of 300 m, six lamps of 80 m and a ring of 500 m radius |
| WON-08 | The Loom | a frame with 120 vertical cables and two shuttle beams |
| WON-09 | The Monument | a blank slab of 200 × 800 × 40 m, nothing on it |
| WON-10 | The Hyperboloid | 24 rings of 16 facets whose radius is `80 + 2 d²` (d rings from the waist) |
| WON-11 | The Wheel | a rim of 64 facets with radius 1 km, a hub and eight spokes |
| WON-12 | The Cistern | a hall of 1.6 km with a floor, a roof and 100 × 100 columns of 4 × 4 × 42 m on a 16 m grid |
| WON-13 | The Drop | **not built**: needs the featherfall shafts of PW.10.14 |
| WON-14 | The Library | six hexagonal galleries (radii 120–420 m) with a floor every 8 m for 30 floors, and eight posts |
| WON-15 | The Bridge City | a deck of 3 km on three piers with 18 towers on it |
| WON-16 | The Nest | 220 beams at random headings and tilts round a core |
| WON-17 | The Stair | one `StairRun` of 200 m wide, 1600 steps (400 m of rise over 600 m) |
| WON-18 | The Furnace Heart | a warm core of 300 m with a lit face in a cage of eight chimneys |
| WON-19 | The Antenna | a mast of 40 × 1 400 m with three pairs of cross arms and a red light |
| WON-20 | The Archive Wall | a wall of 600 × 300 m with 12 × 12 drawers and handles |
| WON-21 | The Mirror | eight towers and their mirror image across a plane of light. **Deviation:** a mirrored cluster of towers, not a duplicated district |
| WON-22 | The Sleeping Builder | a body, a head with a dim eye, six arms, four legs, lying down; the waking is a mover |
| WON-23 | The Funnel | twenty rings from 80 m to 1.9 km of radius rising up; the cone's volume is cleared. **Deviation:** the cell holds 4 km, so the funnel is 3.8 km wide, not 8 |
| WON-24 | The Gate | **not built**: needs a Rampart (PW.10.34) |

### 3W.10 Movers and Builders (PW.10.12, PW.10.13, `worldgen::movers`, `engine::moversys`, `GEN_VERSION` 25)

A mover is a **pure function of the world clock**: `Mover::pose(t_ms)` uses integers only (smoothstep ramps out of 1024) and is periodic, so it agrees across restarts at the same world time and on every thread. An L0 cell holds one with chance 1 in 10 (`MOVER_SITE`, `0x012C`) at an inner L1 cell, of five kinds. The generator emits the parts of the moving piece in its own frame (`movers_in_cell`) and the fixed supports as a feature (`mover_content`: banks, posts, landings, rails and a keep-out of what it moves through, tied to the artery tree).

| Kind | Moves | Period |
|---|---|---|
| **P2-26 moving bridge** | a leaf of 120 m that spans a gap for a third of its cycle, slides 140 m along +x, waits and slides back | 4–8 min |
| **P2-27 drawbridge** | a leaf of 80 m that **lifts** 60 m out of the way and comes down (a lift bridge: chunks and frames do not pitch past 30°) | 4–8 min |
| **P2-28 lift that turns** | a cabin of 10 m that rises 64 m and turns a quarter (16 of 64) as it goes, then sinks back, turning back | 4–8 min |
| **P2-29 tram through a gate** | a car of 24 m on 400 m of line, there and back, turned half a turn (32 of 64) while it passes a ring of light at mid line. **Deviation:** the ring does not change the field (a gravity gate exists only at seams) |
| **P2-30, BLD-01..05 Builder** | a machine of 40 m on rails that creeps 240 m in the first hour of a 24 h cycle and places a beam (40 × 4 × 4 m) every six minutes (the **timed pieces**: BLD-05, a schedule of predetermined geometry, the same at every restart); the frame stands for the other 23 h and then is gone and it begins again; one Builder in three is **half built in its dock** (BLD-03: no moving, nothing placed); a work light on a mast is far-visible (BLD-04) | 24 h |

**Engine.** `MoverSystem::update(seed, player, t_ms)` keeps the movers within 3.5 km at their pose (it runs every fixed step, flying or walking); `gather` gives their solids to the collision world through `WithMovers`, each with its velocity as a platform velocity, so a walker standing on a leaf or a Builder's deck is carried (tests `the_leaf_carries_a_walker`, `a_placed_beam_is_solid`); `draw` keeps one chunk per mover at its pose (rebuilt when the turn changes) and one chunk of the timed pieces that stand. The world clock is the tram system's game time; **a restart resets it** (the save has no clock: P6.2.2), so the agreement across restarts holds for the function and not yet for a saved game. Rotation is quantised to the 64 table headings.

### 3W.11 Strange regions (PW.10.32–PW.10.34, `worldgen::strange`, `GEN_VERSION` 25)

About one L0 cell in fourteen is strange (`STRANGE_SITE`, `0x012D`, keyed by the L0 cell) with one of eleven characters; the **Far Lands** are every L0 cell at least 2²⁰ cells from the origin on some axis; the **Rampart** is a wall along an L0 line (`RAMPART_LINE`, `0x012E`). Geometry draws from `STRANGE_BUILD` (`0x012F`). `Tissue::strange` and `Tissue::rare` carry the region into growth.

| ID | Region | What it does |
|---|---|---|
| STG-01 | The Library | every L1 cell holds a gallery stack (two rings of eight facets, a floor every 16 m for 16 floors) and the quarter is always Archive (QTR-05) |
| STG-02 | The Recursion | mutation 15 (a smaller older form swallowed inside) on every structure |
| STG-03 | The Inversion | the whole L0 cell wears the inverted district of RARE-01 |
| STG-04 | The Grid | every form is one cube of two slots (64 m); set pieces, hubs and variants keep their own. **Deviation:** the cubes are identical but stand where growth puts them, not on a regular lattice |
| STG-05 | Field of Doors | 24 free-standing doorways (two posts and a lintel, 3 × 2.6 m, solid) in every L2 cell (`cell_pieces`) |
| STG-06 | The Labyrinth | the middle L1 cell holds a maze of 32 × 32 rooms of 64 m, walls of 4 × 12 m on a deck, passages a spanning tree drawn by a depth-first walk (1 023 passages, so `2 · 32 · 32 − 1 023` inner walls); its space is kept clear |
| STG-07 | Fog district | the extinction of the atmosphere is six times the regime's (`fog_factor`) |
| STG-08 | Silent district | `silent_at(seed, p)` for the sound system (PW.11) |
| STG-09 | The Slow district | `time_scale_at(seed, p)` is 1 : 10; `moversys` runs its movers at a tenth in it |
| STG-10 | The Miniature | the whole L0 cell wears the dollhouse district of RARE-04 (scale ½) |
| STG-11 | The Far Lands | from 2²⁰ L0 cells out: scale ×4 (RARE-03) and fog three times as thick. **Deviation:** that is all they do; they are 3 × 10¹⁰ m from the origin and are never reached |
| STG-12 | The Rampart | along one L0 line in forty, at one inner L1 row: a wall of 4 km × 2 km × 64 m through every cell of the row; the **Gate** (WON-24) is one cell in eight, with a door of 60 × 120 m, a lintel and a lit sill |
| STG-13 | Builder frontier | half the L0 cells of the region hold a mover and it is always a Builder |

## 4W. L1 in the Web: Frame, Voids, Fill, Arteries (4096 m)

| | |
|---|---|
| **Inputs** | the parent regime and its stack, the cavity `KeepOut` if any, line keys, face keys |
| **Outputs** | Frame pieces (beams, knots), Between content (tethers, drips, spires, …), crust skin pieces, `KeepOut`s for voids, the fill field, ports and arteries (the skeleton) |
| **Children receive** | `KeepOut` for voids, `ForcedPort` for every artery that crosses an L2 face, the fill density per L2 cell, `Zone` for knots and sanctuaries |

Order of computation in a cell: **line features** (§4W.1–§4W.3) → **stack classification of the cell** → **knot** → **fill** (§4W.4) → **ports** → **arteries and ties** (§4W.5). Port derivation (`ports.md` §3) uses only keyed features, never the cell's output.

### 4W.1 Line machinery

*Owners: FRM-01 (and the beam, valley, rift, bore, Road and Spine keys).*

A **line key** names an infinite straight line. Every cell it crosses derives the same line and emits its own clipped piece (§1.4).

#### 4W.1.1 Lines

- A line is `(i, p, q)`: a direction index `i` into the 49-entry table of `primitives.md` §9.1.3, and two integers `p, q` that locate it.
- The **dominant axis** `a(i)` is the axis with the largest `|d|`; ties go to Y, then X, then Z. The other two axes are `u` and `v` in cyclic order after `a` (X, Z for a = Y; Z, Y for a = X; Y, X for a = Z).
- The slopes are `m_u = d_u / d_a` and `m_v = d_v / d_a`. All are ratios of small integers (denominator 1 or 2) and `|m| ≤ 1`.
- A lattice with pitch `P` places the line's intercept with the plane `a = 0`: `u₀ = p·P + jitter_u`, `v₀ = q·P + jitter_v`, where the jitters are drawn from the line's RNG as multiples of 16 m in `[0, P)`. The line is `u(a) = u₀ + m_u·a`, `v(a) = v₀ + m_v·a`, in exact rational arithmetic (`i64` numerator over 2).
- The **line RNG** is `hash(seed, level, I64Vec3(p, q, i), salt)`.

#### 4W.1.2 Finding the lines through a cell

For direction `i`, the lines crossing a cell with range `[a₀, a₁]` along `a` have `u₀ ∈ [u₀min − m_u·a₁ , u₀max − m_u·a₀]` (and the same for `v`), taking the endpoints in the order that gives a non-empty interval. That is at most `⌈(size·(1 + |m|)) / P⌉ + 1` lattice indices per axis. With `size = 4096`, `P = 2048` that is at most 5 per axis, so at most 25 candidate lines per direction and 1225 over all 49. A candidate costs one hash for its existence test. The cell loops the candidates in the order `i`, then `p`, then `q`, each ascending, which fixes the order of its output.

#### 4W.1.3 Spans and extents

A line is cut into **spans** of 32 768 m along `a`, aligned to the world origin (so a span is one L0 cell long along its dominant axis). Each span `s` has its own RNG `absorb(line_rng, s)` which draws, in order: `present` (chance 3/4), `full` (chance 1/2), and if not full the L1-cell-aligned start and end (`a0 = range(0, 8)`, `a1 = range(a0 + 1, 9)` in L1 units within the span). Ends of a partial span are `Knot` or `Stub` (`range(0, 2)`, one draw per end). A full span continues into the next span, whose own draw may end it (that end is a `Stub`).

The **home regime** of a span is the regime of the L0 cell that contains the span's midpoint (`a` at the middle of the span, `u` and `v` from the line equation, floor-divided by 32768). Every property of the span (kind, section, whether it exists, ends) is drawn from the home regime's fields and from the line RNG alone. Cells on the other side of an L0 face therefore agree, although the regime values there differ (`levels.md` §3 only forbids blending, not reading a neighbour's regime, which is a pure function of its key).

#### 4W.1.4 Pitch and existence

| Feature | Level of lattice | Pitch `P` |
|---|---|---|
| Beams | L1 | 2048 m |
| Valleys, rifts | L1 | 4096 m |
| Bores, Roads, Rivers, Rail Abyss, Cable Highway | L0 | 16 384 m |

Beams exist per lattice point with chance `E_i / 4096`, with

`E_i = floor(4096 · cw(c_i) / (n(c_i) · Σ cw))`,

where `cw` is the weave class weight of `regimes.md` §7.2 for the class `c_i` of direction `i`, `n(c)` the number of table entries in the class (3, 6, 4, 36), and `Σ cw` the sum over classes. The expected number of beams crossing an L1 cell is 4 to 10 depending on the weave. At `weave = 0` only the three axes carry beams and `E = 1365` each. Oblique beams exist only when `weave ≥ 16` (`regimes.md` §7.2).

### 4W.2 Frame: beams and knots

*Owners: FRM-01..14 (PW.2.5).*

#### 4W.2.1 Beams

A **beam** is the part of a span that lies in the cell. Its parameters, drawn from the span RNG after the extent draws:

| Parameter | Value |
|---|---|
| Section | 32–400 m, step 16 m. Shape: square, flattened (height ½ of width) or round (cylinder), `pick_weighted` by the character weights (`regimes.md` §10, forms and paths) |
| Kind | `solid girder`, `inhabited beam`, `truss`, `conduit`, `rail beam`, `stepped beam`, `stair beam`, `fallen beam` (FRM-05..12). A stair beam needs `|m| ≤ tan 35°`, so only directions whose rise over run is ≤ 0.7, with `a` horizontal. A fallen beam needs a direction with a vertical component |
| Ends | `Knot`, `Stub` (FRM-04) or continuing |
| Carries | the tag `FRM-xx` of its kind and `FRM-03` for its section shape |

- The beam's solid is `Oriented` with `Frame::Dir { d }`, `origin` on the line, clipped to the cell (`primitives.md` §9). Axis directions give the old trunk case (a vertical beam is exactly a trunk with another section).
- A **mega-bridge** (FRM-14) is any beam, of any kind except `stub`-only, whose line crosses a void or a Between. Nothing extra is generated: the census counts the beams whose span intersects a void.
- **Reservations.** A beam is a `KeepOut` for tissue except for the parts the kind says are inhabited (floors in an inhabited beam, the stair of a stair beam).
- **Beam ties** (§4W.5.4) make every beam in a live cell reachable.

#### 4W.2.1a As implemented (PW.2.5, `worldgen::l1_frame`)

- **Lines.** Direction `i`, intercepts `u0 = p·2048 + 16·below(128)` and `v0` likewise, dominant axis made positive. The line RNG is `hash(seed, 1, (p, q, i), LINE_BEAM)`: draw 1 is `e = below(4096)` (the line's existence value), draws 2 and 3 are the jitters. A span of the line exists iff `e < E_i(weave of its home regime)` and its `present` draw passes (3/4). The home regime is the L0 cell holding the span's midpoint, so every cell agrees (`beams_of_cell`).
- **Span draws** (`absorb(line, span)`): present, full, then for partial spans `a0`, `a1` and two end kinds, then width (`32 + 16·range(0, 24)`), section (square 50, flattened 20, round 30), kind, and the detail seed. A full span's end is `Continue` only if the next span exists and touches the boundary, otherwise `Stub`.
- **Kinds.** Weights girder 30, inhabited 15, truss 20, conduit 10, rail 8, stepped 7, stair 60 (only directions in a vertical plane of X or Z with slope exactly 1/2, so the 64-tread stair of 16 m over 32 m fits), fallen 5 (directions with a vertical part). Stair directions are skew, the rarest class, so stair beams are rare.
- **Pieces** are positioned from the beam's own start, never from the cell, so repeated parts (floors every 6 m, bays of `max(32, w/2)` m, steps, tram cars every 600 m) land identically in every cell. Conduits are seven parallel cylinders; trusses have four chords, frames and a diagonal on each side face (a box in the table direction nearest the brace); inhabited beams are a tube with floors; stepped beams drift up and down on a wave of eight; rail beams carry a rail and trams; stair beams are a staircase of 32 m blocks, each with a stair run.
- **Ends.** Knot ends carry a junction block (FRM-13); stub ends carry three shards (FRM-04). Ends fall on L1 faces, so a solid-agreement test excludes the zones of ending beams.
- **Knot.** Drawn in the inner box `[1, 7)³`, snapped to a beam within one L2 cell (ties: beam index, then cell index), a beam whose end is a knot in the cell winning outright. Hub family weights 40, 20, 25, 5, 10. The structure is a junction block with a beacon.
- **Census.** `Beam::tags()` gives FRM-01, FRM-02 (oblique), FRM-03, the kind and FRM-04 per stub; `is_mega_bridge` gives FRM-14 (any sampled axis point inside a Between).
- **Not in `l1_cell` yet:** assembled with the arteries at PW.2.8 (`GEN_VERSION` 7); beam density per cell is 1–6 at present (mean about 3), below the 4–10 of §4W.1.4 because spans are partial; the census (PW.2.11) may retune.

#### 4W.2.2 Knots

*Owners: FRM-13.*

Every live L1 cell has one **knot** `K`, a point in the L2 grid of the cell, drawn as:

1. The L2 cell: `(range(1, 7), range(1, 7), range(1, 7))` from `hash(seed, 1, cell, KNOT)`, so a knot is never on the outer ring of the cell.
2. **Snapping.** If any beam of the cell has a section axis passing through an L2 cell of the inner box `[1, 7)³` at a Chebyshev distance of at most 1 L2 cell from the drawn cell, the knot moves to the nearest such cell on the beam (ties: lowest beam index, then lowest cell index). A beam whose end is `Knot` always wins, regardless of distance, and snaps to its end cell.
3. The knot's **hub kind** is drawn from the character weights for the hub family (junction hall, lift nexus, machine, gravity furnace, sanctuary site; `PW.3.15`), and its far signature (silhouette, light colour, rhythm) from the cell hash (§11.3 of `the-web.md`).

A knot is **cell-local**: only this cell and its L2 children read it. Neighbouring cells never need to agree on it, which is why it may depend on the beams.

### 4W.3 Voids

*Owners: VOID-01..14 (PW.2.6).*

A void is carved out of crust: it is a `KeepOut` for tissue and fill, and it is not an obstacle for beams or arteries (they cross it as bridges, `§4W.5`).

#### 4W.3.1 Rules for every void

1. **Inner box.** A void lies inside the **inner 6×6×6 L1 cells** of its L0 cell (L1 indices 1–6), so it never touches an L0 face and never crosses into a neighbouring regime. Bores are the exception (see below).
2. **Margin.** Its footprint keeps at least 64 m from the faces of an L1 cell unless it is a line void (valley, rift, bore) whose continuation across the face is derived from the same line key.
3. **Convex pieces.** A void is the union of convex oriented pieces (boxes, cylinders, cone rings). Voids of different keys are separated by at least one L2 cell (512 m) in the Chebyshev metric, **except** line voids with each other, which may cross.
4. **Carries** its tag `VOID-xx`, and every piece is `KeepOut`.
5. **Void fraction.** The expected void volume in a crust is at most 8 % of it. Draw counts are fixed and the odds are scaled by the regime (`1 − fill_crust/255` and `fill_between`).

#### 4W.3.2 Kinds

| Kind | Keyed by | Shape | Parameters |
|---|---|---|---|
| Valley (VOID-01) | line, L1 lattice, horizontal direction (`d_y = 0`) | a box prism along the line, through one crust's full thickness | width 300–2000 m (step 100 m), length 2–24 km, ends rounded by a 4-ring taper |
| Hall (VOID-02..08) | cell | a flat box or ellipse prism inside one crust | footprint 300–1500 m, height 20–120 m (step 4 m); variants: hypostyle forest (grid of columns left standing), vault, cathedral (nave), crush (height 20 m), stepped (terraced floor), drowned (fog) |
| Shaft (VOID-09) | cell | a prism through the crust's whole thickness | 64–512 m, square or round |
| Rift (VOID-10) | line, L1 lattice | a thin prism, tilted from vertical by a tilt index 0–6 | width 20–80 m, length 1–6 km, may step |
| Bore (VOID-11) | line, L0 lattice, horizontal axis or diagonal | a tunnel prism | diameter 12–40 m, length 12–48 km, passing straight through everything. It may cross an L0 face; the same line key gives the same bore on both sides |
| Lungs (VOID-12) | the bore's key | halls spaced `m · pitch` along a bore, `pitch ∈ {2, 3, 4} km` | hall footprint 200–600 m |
| Funnel (VOID-13) | cell | a stack of coaxial rings narrowing downward | top radius 200–1500 m, depth = crust thickness, 8–16 rings |
| Cavity (VOID-14) | L0 rare event | whole L1 cells (§3) | unchanged. It crosses the stack; its rim cells use the crust structure |

Each kind draws in the order: existence (by chance), then the parameters in the order of the table's columns, then the position (on a 16 m step), then variants. A rejected void (it would break rule 3) is absent and the later draws are still made.

**Which crust.** A valley, hall, shaft, rift or funnel belongs to one crust: the crust is drawn by `pick_weighted` over the crusts of the stack with weights proportional to crust thickness, from the void's key. The void's vertical extent follows that crust. Where the crust has the Tilted, Vertical or Curved geometry the void follows `u(p)` (it is an oriented prism in the crust's frame).

#### 4W.3.3 As implemented (PW.2.6, `worldgen::l1_void`)

*Version 10 (PW.2.6b): voids exist in every stack geometry (§4W.3.4). Bores do not depend on the stack.*

#### 4W.3.4 Voids in Tilted and Vertical stacks (PW.2.6b)

Area and valley voids are laid out in **stack space** as for a Flat stack. A **Vertical** stack swaps Y with its stack axis (X or Z) and drops pieces that cannot swap (rifts). A **Tilted** stack turns each box and Y-cylinder into an oriented piece in `Frame::table(yaw, tilt)` anchored at `(L0 centre x, a, L0 centre z)`, `(s, t)` being the stack-space offset from the L0 centre and the vertical extent being scaled by `cos(tilt)` so it follows the crust surfaces; a void whose world bounds leave the inner zone `[4096, 28672]` of the L0 cell is dropped. The mouth of a valley, shaft or funnel piece that touches a crust surface opens the skin plate over its bounding rectangle (`Opening`, plane coordinates when tilted).

**Tags.** The registry rows VOID-03..06 are a garbled list; the mapping used is: VOID-03 any variant hall, VOID-04 hypostyle forest, VOID-05 vault and cathedral, VOID-06 crush, VOID-07 stepped, VOID-08 drowned in fog (a flag for the atmosphere), VOID-02 the plain hall. VOID-14 (Cavity) is the existing L0 event (`cavity.rs`), unchanged.

**Crust keys.** The crusts of an L0 cell's stack are numbered `k`; a void's crust is drawn from its key by `pick_weighted` over the crusts' thickness. `crust_key = absorb(hash(seed, 0, cell, VOID_KEY), k)`. A void's vertical extent follows its crust, clamped to the inner zone of the L0 cell (`y` local in `[4096, 28672)`), with 16 m left under each skin unless the kind opens through it.

**Area voids** (halls, shafts, funnels): one site per crust per L1 column of the inner 6×6 (`x, z` index 1–6). A site draws, in order: `e = below(1000)` (exists iff `e < chance`, with `chance = 1000 · (255 − fill_crust) · 3 / 255` capped at 600), the kind (`pick_weighted([55, 25, 20])` hall, shaft, funnel), then size, variant and position from the same RNG. The footprint stays 256 m inside its L1 column, so two area voids are at least 512 m apart (rule 3). A site whose footprint (grown by 512 m) meets a line void of the same crust is dropped, its draws still made.
- **Hall:** footprint `100 · range(3, 16)` m each way, height `4 · range(5, 31)` m; variant `pick_weighted([20, 15, 15, 10, 15, 15, 10])` plain, hypostyle, vault, cathedral, crush, stepped, drowned. Hypostyle leaves a grid of solid columns (radius 4–10 m, pitch 48–96 m); vault adds three narrower, higher tiers; cathedral adds a nave a third as wide and up to three times as tall; crush forces height 20 m and a footprint of at least 800 m; stepped is 3–5 terraces, each floor 4 m above the last; drowned is a plain hall with the fog flag.
- **Shaft:** 64–512 m (`16 · range(4, 33)`), square or round, the crust's whole thickness (open at both surfaces).
- **Funnel:** `n = 8 + below(9)` coaxial rings, top radius `100 · range(2, 16)`, narrowing to `top / n`, the crust's thickness deep, widest at the top.

**Line voids** (one per axis and lane per crust draw): the six inner lanes of 4096 m across, along X and along Z. Each `(axis, lane)` draws a crust by thickness, a chance of 0.3, then:
- **Valley (VOID-01):** width `100 · range(3, 21)` m, length `1000 · range(2, 25)` m (at most 24 576), start a multiple of 64 m inside the inner zone, the crust's full thickness (open at both surfaces), ends rounded by four tapering boxes.
- **Rift (VOID-10):** width `20 + 4 · below(16)` m, length `1000 · range(1, 7)` m, tilt index `0..=6` and a lean side, in the frame `Frame::table(yaw, tilt)` with `yaw` 0 or 32 (rifts along Z) or 16 or 48 (along X) so that the long side is the frame's `z`. A third of rifts **step**: the second half is offset across by 1.5 widths.

**Bore (VOID-11) and Lungs (VOID-12):** lines on the L0 lattice (pitch 16 384 m on the two other axes, so the line may be at any height) in the eight horizontal table directions. A line exists with chance `125/1000`; a segment of 65 536 m along its axis holds a bore with chance 1/2, of length `1000 · range(12, 49)` m at a multiple of 4096 m inside the segment, radius `6 + below(15)` m; it crosses L0 faces and is the same on both sides. A third of bores have Lungs: halls of `200 + 8 · below(51)` m footprint and `4 · range(5, 31)` m height centred on the axis every `1000 · (2 + below(3))` m.

**Output.** `VoidPiece { piece, solid, fog }`: keep-out pieces (`FeatureKind::VOID`, no collision) and the few solids a void leaves standing (hypostyle columns). `voids_of_l0` gives the whole voids; `voids_of_cell` clips them to an L1 cell. Salts `VOID_KEY` `0x0220`, `VOID_SITE` `0x0221`, `VOID_LINE` `0x0222`, `VOID_BORE` `0x0223`.

### 4W.4 Fill field

*Owner: FILL-01 (PW.2.7).*

Every L2 cell has an integer **fill** `f ∈ 0..=255`, the built fraction the L2 cell aims at. For the L2 cell with L1-local index `(ix, iy, iz)`:

1. **Base.** Take the 8 sample points at the cell's centre column (x and z at the centre, `y` at the 8 sub-slab centres of 64 m). For each sample `t`: `base_t = fill` of the stratum's character if `stratum_at` is `Crust(k)`, `fill_between` of the regime if a Between, and `0` if the sample lies in a void piece. `base = Σ base_t / 8` (integer division, rounding down). For non-Flat stacks the sample column follows `u`.
2. **Noise.** Each L1 lattice corner `c` has `n(c) = below(64)` from `hash(seed, 1, c, FILL_CORNER)`. The L2 cell's noise is the trilinear blend with integer weights over 16: the weights along an axis for index `i ∈ 0..8` are `(15 − 2i, 2i + 1)` over 16 (the first version of this text, `(15 − (2i + 1), 2i + 1)`, summed to 15), and the product is a weight over 16³ = 4096. `n_t = Σ n(corner) · w / 4096`. Neighbouring L1 cells share corners, so `n_t` is continuous across faces.
3. **Result.** `f = base · (192 + 2·n_t) / 256`, clamped to 255. The multiplier is between 0.75 and 1.24.
4. **L2 obligation.** L2 fills its cell to `f` within ±10 % (of the cell volume), and its outer skin covers the far envelope (PW.2.4).

Everything in the formula is integer, depends only on the stack, void pieces and corner hashes, and is the same on both sides of every face.

*As implemented (PW.2.7, `worldgen::l1_fill`):* `fill_field(seed, l1)` returns the 512 values indexed `(iz · 8 + iy) · 8 + ix`; `fill_l2` asks for one L2 cell. The character fill of step 1 is the regime's `fill_crust` until stratum characters exist (PW.2.10, SC-06). Voids are the keep-out pieces of `voids_of_cell`; the standing columns of a hypostyle hall do not count.

### 4W.5 Ports and arteries: the skeleton

*Owners: ART-01..13 (PW.2.8 and PW.2.9). This replaces the lift core, which is no longer in every cell (ART-13).*

#### 4W.5.1 Live cells and ports

- An L1 cell is **live** unless it is a cavity cell. (Betweens are live: they hold arteries as catwalks, tethers and cables. See §4W.5.6 for why.)
- **Every face between two live cells carries at least 2 ports** (`ports.md` §4W). A port is an L2 cell of the face, `(i, j)` in the 8×8 grid of the face, plus an artery type. Ports are drawn from the face key (`ports.md` §3), count `2 + below(3)` (2–4), and distinct cells.
- The port type (`ArteryType`) is drawn from the face key with weights from the stratum characters (`regimes.md` §10), restricted to the types allowed on the face axis (§4W.5.3), and to the environment of the face centre: crust, Between or void (all computed from `stratum_at` and the void pieces, which both cells can evaluate).

#### 4W.5.2 Walks

Each L1 cell has a **tree** `T` of L2 cells, starting as `{K}`. The cell processes, in order:

1. **Port arteries.** The cell's ports, in the order faces `−x, +x, −y, +y, −z, +z` then port id. For port cell `P`:
   1. The **target** is the nearest cell of `T` in Chebyshev distance (ties: smallest index `x + 8y + 64z`).
   2. A **walk** starts at `P`. At each step the candidate moves are the 6 axis moves, the 12 face-diagonal moves and the 8 space-diagonal moves whose destination is inside the 8×8×8 grid and not yet visited by this walk. A move that goes through a void cell is allowed (it becomes a spanning variant, §4W.5.3). Weights: `w_axis = 255 − weave`, `w_diag = weave / 2`, `w_space = weave / 4` (as in `regimes.md` §7.2), multiplied by 4 if the move reduces the Chebyshev distance to the target, else by 1. The pick is `pick_weighted` from the artery RNG `hash(seed, 1, cell, ARTERY)`; draws follow the order of the arteries.
   3. The walk ends when it enters a cell of `T`. If it has not arrived after `4 · dist + 8` steps, a **BFS** over the 6-neighbour grid finishes it from the current cell to the nearest cell of `T` (ties by the lowest index). All cells of the 8×8×8 grid are passable, so the BFS always succeeds.
   4. The cells of the walk are added to `T`.
2. **Loops.** `loops = below(3)`. Each picks two cells of `T` (`below(|T|)` each, the lower index first) at Chebyshev distance of at least 3 and walks between them as above; each loop artery has a type drawn like a port's.
3. **Ties** (§4W.5.4).
0. **Forced arteries** (§4W.5.5) come first. Each forced artery is a straight axis path between its entry and exit port cells. It is joined to `T` by a walk from its cell nearest to `K`, exactly like a port (step 1), and then added to `T`. So lines and spines are in `T` before the other ports.

An artery's **cell list** is the sequence of cells. A diagonal move is recorded with its **supporting axis path** (x, then y, then z), so every artery is also a path of axis steps. The connectivity argument uses only the axis steps. The oblique geometry is drawn inside the supporting cells (clipped to the L2 cell, as everything is).

At every L2 face between consecutive cells of an artery, the L1 cell emits a `ForcedPort` of the artery's type (`primitives.md` §7). Both L2 cells read the same record, so they agree.

#### 4W.5.3 Artery types

| Type | Allowed on faces | Spanning variant (over voids and air) |
|---|---|---|
| Grand stair | any | stair bridge |
| Ramp road | X, Z | ramp bridge |
| Lift trunk | Y | lift on a tether |
| Avenue | X, Z | viaduct |
| Tram tube | X, Z | tube |
| Catwalk web | any | catwalk with cable stays |
| Inhabited beam corridor | any, if a beam passes the port cell, else not offered | beam with floors |
| Ladder chain | Y | ladder cage on a tether |
| Gravity gate | any, only near a well | gate frame |

Which one an artery gets is the only thing PW.2.9 specifies per type. The *spanning variant* is chosen whenever a cell of the artery lies in air (void or Between) at the artery's height. These types are the *arteries*; the tissue's path types (PH-*, PV-*) are chosen by L2 within its own cell.

#### 4W.5.4 Ties

Every feature that is far-visible and has a walkable surface gets a **tie** so it is reachable (the horizon promise):

- every beam in the cell whose kind has a walkable surface (girder, inhabited, truss, rail, stepped, stair, fallen): the tie is a walk from the L2 cell of the beam closest to `K` to `T`;
- every crust skin piece in the cell (one per crust the cell intersects): the walk starts at the skin L2 cell closest to `K`;
- every tether, drip, spire and island (Between content): from its L2 cell closest to `K`.

Ties are processed in the order beams (by line index), crusts (by `k`), Between content (by key), after the loops. Ties add their cells to `T`. A feature that lies in the cell only as a clipped sliver within the outer L2 ring is still tied.

#### 4W.5.5 Forced arteries: Roads and Spines

Roads and Spines (and rivers, bores) are line features at L0 scale. In an L1 cell:

- A **Spine** forces, on every Y face, a port at its L2 column (`§3W.2`) with the types lift trunk and ladder chain (two ports on adjacent L2 cells of the column, chosen by the key). Inside the cell they connect bottom to top as straight axis steps.
- A **Road** (PW.10.1) forces a port on each L1 face its line crosses, at the L2 cell given by its line equation, with the Road's type. Inside the cell, entry and exit are joined by an artery with the Road's type (straight, then axis steps).
- A bore's floor forces a port on the same terms.

Forced ports are part of the face key (the face evaluates the same L0 lines for both cells), so they are in `ports.md`'s count: the count of own ports is `2 + below(3)` **plus** the forced ones. The forced arteries are joined to `T` first (step 0 of §4W.5.2), which is why the later walks attach to them. A forced port must not coincide with an own port; an own port that would is redrawn (`ports.md` §3 keeps the draw count fixed).

#### 4W.6.2a Tilted skins (PW.2.4b)

On a Tilted stack the openings, roofscape, ceiling-scape and seams use the same lattices and draws as on a Flat one, but in **plane coordinates**: `s`, `t` are metres along the x and z axes of `Frame::table(yaw, tilt)` from the anchor (the L0 cell's centre, on the plane), and heights are measured along the frame's y from the plane. Every piece is an oriented piece in that frame with the anchor (an integer world point) as its origin, so two cells derive identical pieces. A cell generates the window of plane coordinates `[centre − 3400, centre + 3400)` and clips to itself. `Opening.plate` marks plane coordinates.

#### 4W.5.7 As implemented (PW.2.8, `worldgen::l1_artery`)

- `face_artery_ports` draws a face's ports from `hash(seed, L1, owner, ARTERY_FACE_X/Y/Z)`: `2 + below(3)` distinct cells `(i, j)` (X faces `(y, z)`, Y faces `(x, z)`, Z faces `(x, y)`), each with a type drawn uniformly from `ArteryType::offered(axis)`. Beam corridors and gravity gates are not offered yet (PW.2.9 adds the context they need), and the stratum weights wait for PW.2.10. *(Superseded by §4W.5.9.)*
- `skeleton(seed, l1)` is `None` for a cavity cell. Otherwise `T` starts as the knot's L2 cell; port walks follow in the order −x, +x, −y, +y, −z, +z; then `below(3)` loops (two cells of `T` by insertion order, a loop whose cells are closer than 3 is dropped; its type is drawn from the offered set of the dominant axis of the displacement); then ties.
- `T` is kept in insertion order. Each diagonal move is expanded into its supporting axis path (x, then y, then z) and those cells join `T`, so every artery is a 6-connected path.
- Walk weights are `255 − weave` (at least 1), `weave / 2` and `weave / 4` for axis, face-diagonal and space-diagonal moves, ×4 when the move reduces the Chebyshev distance. The budget is `4·dist + 8`; a BFS (nearest goal, lowest index) finishes it, also when no candidate move is left.
- Ties: every beam except conduits (`Beam::l2_cells`), every crust the cell intersects (L2 cells whose centre lies in crust `k`), and every tether, drip, spire and island piece (cells its bounds overlap). The start is the candidate nearest to the knot (ties: lowest index). A tie whose start is already in `T` adds nothing. Ties use the catwalk web type.
- **Tuning.** Every number that sets the skeleton's density is a named constant in `l1_artery::tuning` (ports per face, loops, loop span, walk budget, the bias toward the target, which tie kinds exist). Changing one changes generator output (bump `GEN_VERSION`, regenerate goldens). `cargo nextest run -p gen skeleton_census --no-capture` prints the effect: the tree size and path cells by role.
- Not yet: forced arteries (Roads and Spines, PW.10) and the stratum-weighted types (PW.2.9, PW.2.10).

#### 4W.5.8 The assembled Web L1 (PW.2.8b, `worldgen::l1_web::l1_cell`, version 7)

- Primitives, in order: beams and knot (`frame_content`), Between content, crust surfaces and cliffs, then the standing matter inside voids. Voids' keep-out shapes become `KeepOut` constraints. Cavity cells emit nothing.
- Ports: the skeleton's L1-face ports first (`PortKind::Artery { kind }`, rect = the whole face of the L2 cell; floor and width are the L2 tissue's job, PW.3), then one port per L2 face an artery crosses, once per artery type (`FaceKey` at level L2, owner = the lower L2 cell). Each port gets one `ChildConstraint::ForcedPort`.
- **The v6 lift core is gone.** `l1_cell` no longer emits the core, decks, trunks, bridges or the sanctuary, `l1_port` no longer makes the Y-face lift stop, and `l1_cell_v6`, `core_footprint`'s emitters (`core_pieces`, `core_stops`) are deleted. The v6 keyed helpers that the removed code shared (`core_footprint`, `decks`, `trunk`, `shaft`, `chasm`, bridges, rim, `l1_port`) remain as dead code because the v6 L2, L3, validator and engine still compile against them; PW.3 deletes them. Until then the v6 L2 and everything above it run on the Web L1 and are broken by design; the tests that read the v6 output are `#[ignore]`d with a reason (grep `replaced in version 7`).
- `GEN_VERSION` is 7. The L0 golden digest includes `web`. L2 goldens changed because L2 reads L1 ports.

- Measured over 10⁴ cells: `T` holds about 250 of the 512 L2 cells (port arteries about 196 path cells, loops 29, beam ties 9, crust and Between ties 96, with overlaps). That is the cost of 2–4 ports on each of six faces. The L2 tissue (PW.3) may want fewer ports or shorter ones; the numbers are in the PW.2.8 checklist note.

#### 4W.5.9 Port types (PW.2.9, `GEN_VERSION` 8)

- The type of a port is drawn by `pick_weighted` over all nine types in `ArteryType::ALL` order, with weight 1 for an offered type and 0 otherwise, so the number of draws never depends on context. PW.2.10 will replace the 1s by the stratum character's weights.
- Offered by axis: grand stair and catwalk web on any face; ramp road, avenue and tram tube on X and Z; lift trunk and ladder chain on Y.
- **Beam corridor** is offered on a port cell `(i, j)` when a non-conduit beam has an L2 cell at `(i, j)` on the face in **both** L1 cells (the owner's layer 7 and the neighbour's layer 0). Both cells derive the same beams from line keys, so the face key alone decides.
- **Gravity gate** is offered on a face near a gravity well. Wells exist from PW.6.1, so `near_well` is `false` until then and no gate is drawn; the draw path is tested by passing the context in. Item PW.2.9b turns it on.
- Loops use `ArteryType::offered(axis)`, which needs no context.

#### 4W.5.6 Why Betweens are live

The user asked for "an infinite web of routes on every axis" (PRN-03, PRN-04). A Between is therefore not empty: it holds the skeleton threads. The visual cost is low (a catwalk 4 m wide across 4 km of air), and the guarantee does not depend on the stack geometry, the void shapes or the neighbouring regime. The built fraction stays within the targets of the-web.md §4 (2–10 % for the Between).

### 4W.6 Between content and crust surfaces

Tethers (STR-03), drips (STR-04), spires (STR-05), kisses (STR-06), islands (STR-07), tubes, nets, dead machines and the underside lights (STR-08..11), and the crust skins (SURF-01..04, HOR-02) are specified by PW.2.3 and PW.2.4. The rules that bind them here:

- They are keyed by the Between or the crust (a key from the stack), and by a 2D lattice on the surface, so neighbouring cells agree. Pieces are clipped (§1.4).
- Each is "tied" (§4W.5.4) when it intersects the cell.
- Their footprints keep out of the artery cells that carry the `Catwalk web` and `Lift trunk` types only when they are solid (arteries pass through light structures but not through dead machines).
- A crust's **skin** (SURF-04) is a shell of at least 16 m with openings wider than 32 m (HOR-02). The shell is part of L1; L2 covers it to ≥ 90 % (the-web.md §11.2).

### 4W.6.1 What fills a Between (PW.2.3: STR-03..11)

*Code: `worldgen::l1_between`. Output: `WebPiece { tag, primitive }`, the tag being the registry ID, so the census counts what it finds (`web-report`, PW.2.11). Until the Web L1 is assembled (PW.2.5) these pieces are not yet part of `l1_cell`, and `GEN_VERSION` stays 6.*

**Between runs.** For a column of the stack (a point of the plane perpendicular to the stack axis `a`), a **run** is a maximal stretch of Between along `a`, in world metres. `a` is Y for Flat, Tilted and Curved (Curved is Flat until wells exist) and X or Z for Vertical stacks.
- Flat: the segments of `stack(h)` with the cell's own y offset. When the cell above (below) is also Flat, the last (first) segment merges with the first (last) of its neighbour, so a seam Between is one run of depth `D` and its key is that of the lower cell's last Between. Both cells compute the same run.
- Tilted: the segments shifted by the integer `shift` of `regimes.md` §8.2 at the column centre, taken at the three copies `k ∈ {−1, 0, 1}` of the 32 768 m wrap. The ends of a column are embedded by `w/2 · s/64 + 8` m so that the oblique crust surface covers them.
- Vertical: the same along X or Z, merging along the axis with a neighbour that is Vertical on the same axis.
- A **run key** is `hash(seed, 0, home, BETWEEN_KEY)` absorbed with the index of the Between in its stack. All draws below come from `hash(seed, 1, (p, q, run key as i64), salt)` for a site `(p, q)` of the run's lattice, never from the cell, so every L1 cell that sees the site draws identically.
- **Homes.** Every lattice pitch below divides 4096 m, so a site never crosses an L1 face across the stack and never crosses an L0 side face either. The home of a site is the L0 cell of the generating L1 cell, and a run comes only from that cell's stack (plus the merge above). Across an L0 side face the neighbour's own stack differs and nothing crosses: that is the section cliff (SURF-03).
- **Run parameters.** A run carries everything its draws need, taken from the cell that owns its key (for a merged seam run, the lower cell): `fb = fill_between`, its kind and its flooding. Two cells that share a seam run therefore draw identically.

**Sites.** A run of kind `K` has a lattice of pitch `P_K` in the plane, in world coordinates:

| Kind | `P` | Content |
|---|---|---|
| Shallow (BTW-01) | 128 m | a forest: thin tethers (width 16–64 m) at the calibrated chance of the site draw |
| Normal | 1024 m | the standard mix (below) |
| Abyssal (BTW-02) | 2048 m | weights tether 200, island 50 and nothing else (the table below with the others at 0); nets at chance 0.1; underside lights ×2 |
| Pillared (BTW-03) | `256 · 2^below(3)` m (256, 512 or 1024), drawn per run | a hypostyle: **every** site holds a tether of the run's one width `w ∈ {32, 48, 64, 96}` m (`w ≤ P/2`), centred on the lattice, nothing else |
| Webbed (BTW-04) | 2048 m | no tethers; nets only: 2–4 stacked layers per site at chance 0.7; tubes allowed |

A site's footprint is a square of its width inside its own lattice cell; the centre is jittered by a multiple of 16 m so that the footprint never leaves the lattice cell, so sites never overlap and a cell only reads the lattice cells that overlap it.

The **site draw** (Normal and Abyssal), in this order: `e = below(1000)`, and the site exists iff `10·e < 1000·fb`, a chance of `min(1, fb / 19.9)`: `e · 199 < 10 000 · fb`. The constant is calibrated, not guessed: with every site present the width classes below fill 7.8 % of a Normal Between, so `fb / 255` is reached at `fb / 19.9`, and the built share follows `fill_between` across 2–20 (the test `built_fraction_tracks_fill_between`). Above 20 the lattice is full and the share saturates at about 8 % (Shallow 8.4 %), a little under the 10 % end of the target; then the type from `pick_weighted([450, 120, 120, 60, 80, 20])` (tether, drip, spire, kiss, island, machine; Abyssal `[200, 0, 0, 0, 50, 0]`); a machine needs `P ≥ 1024` and a Between of at least 8 units, otherwise it is a tether; then the width class (below), the section, the jitters in `s` and `t`, then the type's own draws. The forest uses `e · 196 < 10 000 · fb`, calibrated in the same way.

**Tether (STR-03).** Width `w` from the classes 50–100, 100–200, 200–400, 400–800 m with weights 15, 25, 30, 30, a multiple of 2 m, at most `0.8 P`. Section: round (60 %), square (40 %). `w ≥ 120` and square: hollow with chance 0.3 (walls of thickness `max(8, w/12)`, four boxes, open at both ends of the run), and a hollow one holds a **ramp switchback**: flights of rise `(w − 2t)/4` alternating between two lanes of 6 m with a landing at each end, up to 40 flights per cell. It spans the run from one crust to the other (`lo..hi` of the run, extended into the crust by 8 m at both ends). Flooded runs add a lamp ring (an EMISSIVE thin cylinder, radius `w/2 + 1`, 2 m tall) every 128 m.

**Drip (STR-04) and spire (STR-05).** A stack of `n = 4 + below(5)` tiers of square boxes. Total height `H = D · (30 + below(56)) / 100` (a spire from the lower crust, a drip from the upper crust). Tier `i` has width `w0 · (n − i) / n` rounded to 2 m with `w0 ∈ [100, 0.8 P]` (multiple of 2 m, same classes as a tether but capped), height `H / n`, and is centred on the site centre (offset by `±w0/12` from the tier before: `below(3) − 1` per tier). A tip needle (a cylinder of radius `max(1, w0/40)`, length `H/4`) ends it. The drip is the mirror image, hanging from the underside.

**Kiss (STR-06).** One spire of height `m` and one drip of height `D − m`, `m = D · (40 + below(21)) / 100`, whose last tiers have the same width `t = max(8, w0 / 5)` and meet at the waist: a **waist box** of width `t`, 8 m tall, joins them, and a **waist stair** (a `StairRun` of 16 m rise over 32 m, 64 steps, 8 m wide) climbs beside it from the spire's last tier to the drip's.

**Island (STR-07).** A stem (cylinder, radius `4 + below(21)` m) from the lower crust (70 %) or the upper crust (30 %, hanging) to a height `D · (40 + below(41)) / 100`, topped (or hung) by a platform box of width `120 + 2·below(191)` m (capped at `0.8 P`), thickness `10 + below(21)` m, plus `1 + below(3)` boxes of 8–24 m on it (sheds). The stem never reaches the other crust (that is what makes it stranded).

**Dead machine (STR-10).** A horizontal cylinder (axis X or Z by a draw) of radius `100 + 2·below(101)` m and length `2.4 × radius` to `6 × radius`, standing on 4 legs (boxes of width `radius/6`) down to the lower crust (or hung from the upper by the same legs when the draw says so), with `2 + below(3)` flanges (radius +30, thickness 24) and a hub cap at each end. The machine sits in the middle third of the run. Its radius is capped at `0.35 · D`.

**Tube (STR-08), run-level.** Per run: `t_n = below(3)` tubes (Normal), `below(2)` (Abyssal and Webbed), none otherwise. A tube is keyed by `(home, run, t)`: an axis (X or Z), the position on the other horizontal axis (a multiple of 64 m, in `[4096, 28672)` of the L0 cell), a height at `D · (20 + below(61)) / 100` above the lower crust, and its ends `a0 < a1` in the L0 cell (`a0` in `[2048, 16384)`, length `4096 + 64·below(128)`, capped inside the cell). It is a cylinder of radius `8 + below(23)` m with rings (radius +3, 4 m thick) every 256 m, stations (boxes `80 × 40 × 40` m) at the ends, and pylons (cylinders of radius 3 m) from the tube up to the upper crust every 1024 m. Half the tubes are **open gantries** instead: a rail box `6 × 3` m and a tram of `3 + below(6)` cars (boxes `30 × 8 × 8` m, material `TRAIN_LIGHT` on the window strip) spaced 40 m, at a position on the rail drawn from the key. L0-scale lines do not cross L0 faces (they end at their stations), so cells never need to agree on them.

**Net (STR-09).** Patches on a lattice of 2048 m (in the plane), each a square of side `512 + 32·below(41)` m placed inside its lattice cell, at a height `D · (10 + below(81)) / 100`, with cables of radius 1 m along X and along Z every 32 m. Chance: Normal 0.25, Webbed 0.7 (2–4 layers, spaced `D / 6`), Abyssal 0.1.

**Underside lights (STR-11).** On the underside of the upper crust of every run: a 64 m lattice of windows. A 256 m block draws a density `dens ∈ {0, 0, 2, 4, 8, 14} %`; a site draws a window if `below(100) < dens` (doubled in Abyssal, tripled when flooded). A window is an EMISSIVE box of 8 × 5 m across the stack and 1 m along it, hanging from the surface (`a ∈ [hi − 1, hi)`), at an offset of `4·below(14)` m in both directions inside its 64 m lattice cell. Only cells whose range holds the surface (`hi`) draw them (Tilted stacks test each site).

All pieces are clipped to the cell (`§1.4`). Collision: tethers, drips, spires, islands, machines, tubes and pylons collide; cables, lamp rings, windows and nets do not. Flags: `FAR_VISIBLE` for anything wider than 24 m. Materials: CONCRETE for solids, METAL for machines, tubes and rails, PIPE for cables, EMISSIVE for windows and lamps, TRAIN_LIGHT for the tram windows. Features are keyed by the run, never by the cell, so a feature that crosses an L1 face is derived identically on both sides (invariant 4); the seam test in `l1_between` compares the solid on both sides of a face.

**Budget.** A cell reads up to nine runs' lattices; the Normal case is 16 sites per run. The underside lights are the large item: at most 4096 candidate hashes per run and about 5 % of them emit. Meshing cost is measured when the Web L1 lands (PW.2.5).

### 4W.6.2 Crust surfaces and the skin (PW.2.4: SURF-01..04, HOR-02)

*Code: `worldgen::l1_surface`. Same keys and rules as §4W.6.1: everything hangs on a Between run and a lattice site, so every cell derives the same feature.*

Every run has two surfaces: its floor `lo`, the top of the crust below, and its ceiling `hi`, the underside of the crust above. Each surface has one **kind**, drawn from the run key:

| Surface | Kinds (chance) |
|---|---|
| Floor `lo` | Roofscape (70 %, SURF-01), Skin (30 %, SURF-04) |
| Ceiling `hi` | Ceiling-scape (65 %, SURF-02), Skin (35 %, SURF-04) |

**The skin plate.** Every surface carries a plate of 16 m, the **skin**: `[lo − 16, lo)` under a floor, `[hi, hi + 16)` over a ceiling. It is one box per cell cut into rectangles around the **openings** (below). In a Tilted stack the plate is one `Oriented` slab in the frame of the stack (`Frame::table(yaw, tilt)`). Its surface and the stack agree because `tilt_shift` uses the same `sqrt` as the frame's axes (`regimes.md` §8.2). A Tilted surface has **no openings and no dressing** yet (rotated lattices: PW.2.4b). Flat and Vertical stacks have both. **Wrap.** A Tilted stack's last Between and its first form one run across the wrap (`levels.md` §4W.6.1 runs), so no plate stands between them.

**Openings (HOR-02).** Large features are L1 data: every opening wider than 32 m in a skin is decided here, and L2 may add only narrower ones (§5W.7). A lattice of pitch 1024 m in `(s, t)`; a site holds an opening with chance 0.35 (Roofscape and Ceiling-scape surfaces) or 0.2 (Skin surfaces, which are near blank); the size is `8 · range(4, 26)` m (32–200 m) square-ish, or for a Skin surface `8 · range(8, 51)` m (64–400 m); the position is any multiple of 8 m inside the lattice cell that keeps the opening inside it. The openings of a cell are available as data (`surface_openings`) for the children: they are `KeepOut`-free (they must stay open) and L2 may route through them.

**Roofscape (SURF-01).** On lots of 64 m: a building with chance 0.55 (skipped if it touches an opening): footprint `8 · range(2, 8)` m, height `4 · range(1, 8)` m, one in twenty a tower of `60 + 4 · below(16)` m; a **terrace** (a second, set-back tier of footprint −12 m and `4 · range(1, 4)` m) on 60 %. **Antenna forests:** a block of 256 m draws one with chance 0.25: `6 + below(15)` masts (cylinders of radius 1 m and height `20 + below(101)` m) at positions inside the block, 30 % of them with a BEACON box of 2 m on the tip. Buildings wider than 24 m are `FAR_VISIBLE`.

**Ceiling-scape (SURF-02).** Coffers: ribs hanging from the plate, 6 m wide and 10 m deep, along `s` and along `t` at every 64 m; hanging machinery (a box of `8 · range(1, 5)` m, 12–40 m deep) at chance 0.3 per 256 m block; pipe runs along `s` every 128 m (cylinders of radius 2 m, 14 m below the plate) at chance 0.5; hanging lamps (an EMISSIVE box of 2 m on a 1 m chain cylinder, 6–30 m long) at chance 0.1 per 32 m lattice site.

**Skin surface (SURF-04).** Nothing but the plate and its panel seams (1 m wide strips, 1 m deep, along both axes every 128 m) and the openings.

**Section cliff (SURF-03).** Where a crust is cut by a face, here an L0 side face on which the neighbour's stack has a Between (or the other side of a rift, PW.2.6, which calls the same generator), the cut is a **wall of floors**. Each cell on an L0 side face (`coordinate mod 8 ∈ {0, 7}` across a side) samples its face on a 64 m grid with both stacks: a sample is **exposed** when the point just inside is in a crust and the point just outside is not. Exposed samples are merged (down each column, then across equal columns) into rectangles, and each emits:
- a wall plate 5 m thick set back 3 m from the face (`[F − 8, F − 3)`);
- floor slabs, one per 4 m of height: 1 m thick, 3 m deep (`[F − 3, F)`), over the width of the rectangle;
- lit windows on every other floor (every 8 m): EMISSIVE strips of `8 · range(2, 6)` m × 2 m × 1 m on the wall, one draw per 256 m block of the face with chance 0.3. The cliff's window draws are keyed by the face and the floor, so a cut face drawn from either cell gives the same lights.
Nothing leaves the cell. A cliff face is generated only on the side that holds the crust, so each cut is drawn once.

**Collision and flags.** The plate, buildings, ribs, wall plates and floors collide; antennas, lamps, seams, windows and pipes do not. `FAR_VISIBLE`: the plates, buildings over 24 m, cliff plates and floor slabs.

**Budget.** A roofscape cell holds up to ≈ 5 000 pieces (4 096 lots), and only the cell that holds the surface draws it. A full cliff face holds about 3 000. Both are measured when the Web L1 lands (PW.2.5).

### 4W.7 Salts and draw order

Salts added in the L1 block (`0x0200–0x02FF`), append-only:

| Salt | Value | Key | Use |
|---|---|---|---|
| `LINE_BEAM` | `0x0210` | `(p, q, i)` | beam line properties and span draws |
| `LINE_VALLEY` | `0x0211` | `(p, q, i)` | valley and rift lines |
| `KNOT` | `0x0212` | cell (L1) | knot draw |
| `VOID_CELL` | `0x0213` | cell (L1) | halls, shafts, funnels |
| `FILL_CORNER` | `0x0214` | lattice corner (L1) | fill noise |
| `ARTERY` | `0x0215` | cell (L1) | walks, loops, ties |
| `ARTERY_FACE_X/Y/Z` | `0x0224–0x0226` | owner cell (L1) | face ports and their types |
| `BETWEEN_KEY` | `0x0216` | cell (L0) with the Between index absorbed | identity of a run (§4W.6.1) |
| `BETWEEN_SITE` | `0x0217` | `(p, q, run key)` (level 1) | the site draw |
| `BETWEEN_LINE` | `0x0218` | `(t, 0, run key)`; `(−1, −1, run key)` for the count | tubes |
| `BETWEEN_LIGHT` | `0x0219` | `(block p, block q, run key)` | density of a 256 m block of underside windows |
| `BETWEEN_NET` | `0x021A` | `(p, q, run key)` | net patches |
| `BETWEEN_WINDOW` | `0x021B` | `(p, q, run key)` | one underside window |
| `SURFACE_KIND` | `0x021C` | `(which, 0, run key)` | kind of a surface (§4W.6.2) |
| `SURFACE_OPENING` | `0x021D` | `(p, q, run key + which)` | openings of a skin |
| `SURFACE_LOT` | `0x021E` | `(p, q, run key + which)` | roof lots, ceiling blocks and lamps |
| `CLIFF_WINDOW` | `0x021F` | `(block, row, face key)` | cliff window blocks |
| `LINE_L0` | `0x0120` | `(p, q, i)` (level 0) | Bore, Road, River lines |
| `SPINE` | `0x0121` | ColumnKey (L0) | spine placement |
| `WELL` | `0x0122` | cell (L0) | gravity well centres (`gravity.md`) |

Draw order in an L1 cell (version 7): beams (§4W.2), knot, voids, fill, then arteries (§4W.5.2). Everything the L2 children need is passed as `KeepOut`, `ForcedPort`, `Zone` and the fill field.


> **Web redesign.** The L2 content for version 7 is §5W below. This section describes versions 1–6.

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

**As implemented** (`worldgen::l2`, `l2_walk`, P0.8.4): for each walkway port, each cell builds its half of the skywalk (`Skywalk`): a 0.5 m `GRATING` slab 8 m long from the face at the port's floor, then a `StairRun` (rise 3 Q, tread 5 Q) descending away from the face to the deck top, ending up to 2 Q below the slab (a step). Heights are multiples of 4 m, so every stair is over 3 m and no ramp is needed. The corridor is a `KeepOut`, and masses keep 8 m from it (P0.8.3b). Skywalk halves use only ports and L1 data, so the walkway-port connectivity is by construction; the validator confirms it in tests. Mass entrances, spanning trees and loops (P4.1.6) are in `worldgen::l2_entrance` and `l2_links`, as specified in §6.2. Entrances are `PortKind::Entrance { plane }` ports after the face ports, each with a `ForcedPort`; links are `KeepOut` clearance paths. Two deviations: nodes at lift-shaft and stair-tower openings are not yet linked, and a skywalk's foot node is the deck 2 m past the end of its stair.

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
- The cell's metadata flags it. **As implemented** (`worldgen::sanctuary_room`, P5.2.1): a 24 × 24 m room 4 m inside the zone (so the door always opens onto deck), `WARM`, 6 m high with a 0.5 m roof; the door (3 × 2.75 m) is on a side drawn from `SANCTUARY_ROOM`, and a 12 m wide opening (sill 1 m, top 4 m) faces the opposite way. Props are L4 modules (`l4::sanctuary_modules`): two benches, four plants, two warm lamps.

### 5.6 Vista `[VISTA]`

- An L2 cell with an L1 `KeepOut` void (shaft, chasm or cavity) within 64 m of one of its walkable surfaces gets a vista with `chance(0.25)`.
- **As implemented** (`worldgen::l2_vista`, P0.8.7): draws are the 0.25 chance, a starting side, then eight placement offsets. For each shaft or chasm of the parent and each deck top in the cell, the four sides are tried from the starting side: a 16 m wide strip of deck 8 m behind the void's edge must be fully supported and clear, then the balcony (8 m over the void, a 0.5 m `GRATING` slab flush with the deck) and a corridor (16 m wide, 8 m high, 16–64 m long over the void) must be free of every L1 solid and `KeepOut` except the void's own, of skywalk corridors and lift shafts, and inside the 72 m margin of the cell. The first fit wins; if none does the cell has no vista. The corridor is a `KeepOut`; balcony and corridor form a `Zone { Vista }`; the cell sets `MetaFlags::VISTA`; masses avoid the zone.
- The vista is a walkable platform at the void's edge with a 16 m wide `KeepOut` corridor facing the void, and a `Zone { Vista }` for L3. L3 must keep the corridor open.

## 5W. L2 in the Web: the Tissue (512 m)

*PW.0.4. Version 7. This replaces §5 once PW.3.x lands; §5 describes versions 1–6. Catalogue entries (each form, support variant, path type, mutation, threshold, hub, set piece) are specified by the items that own them (PW.3.x), against the framework below. Architecture: `the-web.md` §6.*

| | |
|---|---|
| **Inputs** | the parent L1 output: `KeepOut` (voids, beams, artery corridors), `ForcedPort`s (artery crossings and Roads), the fill field value `f`, the knot (if in this cell), the stack query and the stratum `Character`; the cell's frame (`regimes.md` §7.4); face ports from the neighbour-independent keys |
| **Outputs** | structures (`ENVELOPE` masses with floors planned for L3), routes (final geometry), set pieces, thresholds, dressing, ports (including `Entrance`), `ForcedPort`s, `Zone`s |
| **Children receive** | an `Envelope` per mass, `ForcedPort`s for every entrance, `Zone`s for set pieces and views, `KeepOut` for routes |

### 5W.1 The slot lattice

Routing, placing and measuring happen on a lattice in the cell's **frame** (`regimes.md` §7.4): origin at the cell centre, axes `ex, ey, ez` of the frame.

- Pitch **32 m**, so the lattice is 16 × 16 × 16 = 4096 **slots** in frame coordinates. A slot is `(sx, sy, sz)` with `0 ≤ s < 16`, covering a cube of 32 m in the frame. Slots whose centre is outside the L2 cell (when the frame is rotated) are **absent** and not usable. At a quarter-turn frame or `Id` all 4096 are present and the lattice is aligned with the cell.
- `Scale` of the stratum (SC-05) multiplies the pitch by the scale ratio, rounded to a whole number of 16 m (so a ×½ stratum has a 16 m lattice of 32 × 32 × 32 slots; ×4 has 128 m and 4 × 4 × 4).
- Each slot has an **occupancy state**: `Free`, `Reserved` (parent `KeepOut`, frame, artery corridor: no tissue may enter), `Route`, `Built` (inside a structure), or `Engulfed` (a route slot inside a structure).
- Slot ↔ world mapping is by `primitives.md` §9.1.4; occupancy is decided on the **integer AABB** of the shapes (§9.5), not on the float solid.
- **Port stubs.** Ports of the L2 faces (cascaded or forced) are placed in world axes, not in the frame. A port is joined to the nearest slot centre by a short **stub** along world axes, at most one slot long, in the order x, then y, then z. The stub counts as `Route`.

### 5W.2 The four steps (order is what makes connectivity hold)

The cell runs the steps in this order, with its RNG `hash(seed, 2, cell, TISSUE)` (a draw counts as always made, even when a step is empty).

#### Step 1: Reserve

Mark `Reserved`: every parent `KeepOut` (voids, cavity bites, beam bodies that are not inhabited, artery corridors), every frame piece, and the knot's hub footprint (`Zone`). A reserved slot can still be **crossed** by a route, but only as a bored passage (below). The sanctuary zone, if any, is reserved as before (`§5.5`).

#### Step 2: Route

1. **Anchors.** The cell's route anchors are, in order: its ports (cascade, forced, own, sorted by face then id), the knot (if present), artery crossing points, and set-piece anchors drawn here (`anchors = below(3)` extra places such as a lift lobby or a view point; they are placed on free slots at the edge of voids first).
2. **Tree.** Connect the anchors with walks on the lattice. A tree `R` starts as the first anchor's slot. For each following anchor, walk to the nearest slot of `R` (Chebyshev, ties by index). The walk uses 6 axis moves and, when the stratum weave allows, face-diagonal moves; moves into a `Reserved` slot cost 8 instead of 1, and into an absent slot are not allowed. The walk is a weighted random walk with a step budget, finished by a deterministic Dijkstra on the lattice (ties: lowest slot index). The lattice is complete (apart from absent corner slots, which never disconnect it), so the walk **always succeeds**; a route through a `Reserved` slot is realised as a bored passage: a tunnel (PH-11) or crawl duct (PH-12) through the reserved solid, with a bulkhead threshold on each side.
3. **Segments.** The walk is cut into segments at corners, at height changes and at changes of surrounding state (free air, over a void, beside a structure). Each segment picks a **path type** from the stratum's weights (`regimes.md` §10), filtered by context (table in §5W.5).
4. **Loops.** `below(3)` extra walks between random slots of `R` that are ≥ 4 slots apart, so the route net is not a bare tree.
5. **Engulfable marking.** Each segment is marked *engulfable* with chance `engulf_p` (1/8 by default; the character may change it). An engulfable segment is a candidate to disappear into a structure in step 3.

#### Step 3: Grow

Structures are placed until the built fraction reaches the fill density `f` (the fraction of present slots that are `Built` or `Engulfed`) within ±10 % of the cell volume. Each attempt is a fixed set of draws:

1. **Support** (`SUP-01..11`) by `pick_weighted` over the character's support weights, restricted to those with a free support surface (§5W.3).
2. **Form** by `pick_weighted` over the character's form-family weights, then a member of the family (§5W.4).
3. **Mutations**: `below(3)` of them (0–2), each picked by the mutation weights (§5W.4.2).
4. **Scale and frame.** The form's scale (the character's) and the frame: the cell frame, or for a `Colliding` support a second frame from the neighbouring grain (§5W.3).
5. **Place.** The support gives an anchor surface point and an orientation. The structure's volume is rasterised on the lattice. It is accepted if:
   - it overlaps no `Reserved` slot and no `Built` slot, except where the support is `Colliding`, `Embedded` or `Nested` and the overlap is intended;
   - it overlaps a `Route` slot only if the segment there is engulfable (then those slots become `Engulfed` and the segment gets `Entrance`s, below);
   - it lies wholly inside the cell with the usual 8 m margin (tissue structures are not clipped across faces; only L1 keyed features cross faces);
   - its access can be made (below).
6. **Access.** The structure has at least one **entrance** on its support contact, and **more entrances** when it is large (one per 30 m of footprint, up to 8), including where an engulfed route enters and leaves. Each entrance is joined to `R` by a walk like step 2 (a complete lattice, so it succeeds), unless the structure is itself on a route. Entrances become `PortKind::Entrance` ports with a `ForcedPort` for L3 (`levels.md` §6.6). A form that is **sealed** by its own definition (a blank monument face, a sealed capsule) is marked `NO_ENTRANCE` and is exempt; sealed structures must still be tied to the support by contact.
7. **Accretion.** A new structure exposes new support surfaces: its roof, underside and sides. They join the surface list for the next attempts. This is how structures grow on structures.

The loop stops when `f` is reached, when 64 attempts have been made, or when 16 consecutive attempts have been rejected. Every attempt draws a fixed number of values (rejected attempts still make all their draws), and the cell's output depends on the attempt order only.

**Engulfed routes (TIS-04).** If a structure overlaps a route segment marked engulfable, the slots become `Engulfed` and the segment is rewritten: the segment ends at the structure's boundary on both sides, with an `Entrance` at each. Inside, L3 must connect the two entrances (`levels.md` §6.8). This is where several entrances on several storeys come from: the route's entry and exit are at the heights of the route there. A route segment that is **not** engulfable is a hard obstacle for structures.

#### Step 4: Dress and pay off

1. **Edges.** Railings, ladders for drops that need them, stubs and thresholds (§5W.6). The edge rules of §5.4 apply (railings are 1.1 m, with the same drop test).
2. **Skin.** The skin pass of §5W.7.
3. **Dead ends.** Every leaf of `R` that is not a port, an anchor or a knot is a **dead end**. Each must end at a **payoff**: a set piece of the catalogue (§5W.8), a view, a light, or a lift. The pass tries, in order, to place:
   1. a **set piece** drawn by `pick_weighted` over those that fit in the free slots around the leaf (size from the catalogue);
   2. a **view**: if a ray from the leaf into free or void slots runs at least 200 m (or reaches the cell face, where the neighbouring cell's occupancy is unknown, so a free ray to the face counts only when the face cell is a void, a Between or a ported face), the leaf is turned into a balcony or window facing it. The ray is cast over the integer lattice;
   3. a **lift**: a one-shaft lift joining the leaf to a different level of the tree, if the slot column fits;
   4. a **light**: a lamp (a `Mass` of one slot with the `EMISSIVE` material) at the end. This always fits, because the leaf slot itself is free. It is the guaranteed last resort, so the rule can always be met.
   The validator checks the rule: no leaf without one of the four.
4. **Loops closing.** If a leaf lies within 2 slots of another route slot (not in the same segment), the pass may instead join them with probability 1/2; the draw is always made.

The rule has the id `PRN-05` and each payoff carries its own id in the census.

### 5W.3 Supports: the framework

*Owners: TIS-06, SUP-01..11 (PW.3.2); variants SV-01..26 (PW.3.3, PW.3.4).*

A **support** is a way a structure attaches to its surroundings. It is a record:

```rust
pub struct SupportSpec {
    pub id: SupportId,                // SUP-01..11
    pub surface: SurfaceClass,        // what it attaches to (below)
    pub up: UpRule,                   // how the structure's up is set (below)
    pub body_axis: BodyAxis,          // vertical, horizontal along the surface, along a beam, radial
    pub variants: &'static [VariantId], // SV-xx, drawn by pick_weighted
}
```

| Support | Surface class | Structure's up | Body axis |
|---|---|---|---|
| Standing (SUP-01) | upward face of a beam, a roof, a knot, or the ground of a slope | local up | vertical |
| Hanging (SUP-02) | downward face of a crust, a beam, a bridge | local up, "floors normal inside", the structure is hung below | vertical, growing down |
| Spanning (SUP-03) | two surfaces facing across a gap | local up | horizontal, between the anchors |
| Sideways (SUP-04) | a vertical face of a structure or trunk | local up, **long axis horizontal** | horizontal, away from the face |
| Collar (SUP-05) | the side of a trunk or tether | local up | ring or helix around it |
| Embedded (SUP-06) | the inside of a beam or a thick wall | local up | along the host |
| Suspended (SUP-07) | a cable from above | local up | hangs from cables |
| Piled (SUP-08) | any surface, many small pieces | random per piece | any |
| Nested (SUP-09) | the inside of a hall, atrium or hollow tether | local up | vertical |
| Colliding (SUP-10) | none: placed in a second frame | local up in the second frame | vertical |
| Radial (SUP-11) | the surface of a gravity well | away from the well | radial |

- **Local up** is the frame's `+ey` (or `+Y` when the cell's frame is `Id`), and with gravity wells (`gravity.md`) the local down.
- A structure on a **Hanging** support is not inverted: its floors stay normal and it is hung from the surface above. An **inverted** form (MUT-01) turns the form's own geometry upside down on top of any support.
- **Colliding** places the second structure in the *neighbouring grain's frame* (`regimes.md` §7.3): the frame of the grain cell that the neighbouring cell's centre lies in, or, within a single grain, a frame turned by `yaw + 8` headings (45°). The wedge between the two is allowed to remain as a space; overlaps are intended, and slots in the overlap are `Built` by both.
- The support variants SV-01..26 are modifiers of the support: each lists how it differs (stilts, a viewing pod, a broken edge, …). A variant is picked by `pick_weighted` with equal weights unless the character sets otherwise, and it may add extra pieces (stilts) or change the anchor (kissing).
- Surfaces come from a **surface list** the cell maintains: each entry is `(slot or face, normal, class, owner)`. The list starts with the L1 frame pieces, the cell's walkable L1 surfaces (artery floors, crust skin), the voids' rims, and each route. New surfaces are appended as structures are placed (accretion).

### 5W.4 Forms and mutations: the framework

#### 5W.4.1 Forms

*Owners: FORM-01..23 (PW.3.5–PW.3.7).*

A **form** is a pure function of a RNG, a scale and a frame that returns a `FormOut`:

```rust
pub struct FormOut {
    pub id: FormId,                   // FORM-xx
    pub pieces: Vec<Primitive>,       // in the form's frame, ENVELOPE flags as in §5.1 where L3 refines them
    pub floors: Vec<FloorPlan>,       // storeys: height, plan rect, level
    pub entrances: Vec<EntranceCand>, // candidate openings (position, facing, floor)
    pub footprint: SlotBox,           // for rasterising
    pub sealed: bool,
}
```

- Sizes are in the form's table (PW.3.5–7) and scale with the character's scale ratio, except that doors, handrails and steps do not scale.
- A form uses only the shapes of `primitives.md` (§3 and §9): boxes, cylinders, ramps and stair runs, oriented as needed. Curves are faceted (`§9.3`).
- **Floors.** The floor height is 4 m by default (`L3` rule), 3 m to 6 m by form. A form lists its floors so that `L3` has the storey plan.
- Forms obey the envelope rule of §5.1: the structure drawn at L2 is an envelope that L3 refines, and L3 must cover its faces (75 %) and, at the crust surface, the skin rule of §5W.7.
- **Inverted, sideways and tilted forms** keep their floors normal to local up: an inverted ziggurat has its terraces facing up (walkable); a sideways tower is a normal tower laid along its axis, with the floor orientation turned by 90° so that, inside, the floors are still horizontal. A tilted form is turned by a frame tilt, and its floors are still horizontal.

#### 5W.4.2 Mutations

*Owners: MUT-01..15 (PW.3.8).*

A mutation is a function `FormOut → FormOut` (or on the frame). Up to two are applied, in the drawn order:

| Group | Mutations |
|---|---|
| POSE | inverted, tilted, twisted |
| DAMAGE | broken, overgrown, swallowed |
| COPY | doubled or mirrored, stretched |
| HOLLOW | hollowed, caged, pierced |
| LIGHT | dark, lit |
| SCALE | scaled (×4 or ×¼), fused with a neighbour |

(The full id list, MUT-01..15, is in the registry.) `Swallowed` (MUT-15) is the mechanism of the "older structure inside newer mass": it embeds a copy of a second form inside the first, drawn from an older stratum's palette. `Pierced` reserves a route through the form and makes it a `bored through` route segment.

### 5W.5 Path types: the framework

*Owners: PH-01..20 (PW.3.9, PW.3.10), PV-01..19 (PW.3.11, PW.3.12), MOD-01..09 (PW.3.14).*

A path type is a record:

```rust
pub struct PathSpec {
    pub id: PathId,                  // PH-xx or PV-xx
    pub needs: PathNeed,             // Flat, Rise(min,max), Span(min,max), Interior, Gravity
    pub width: (Q, Q),               // range of the clear width
    pub build: fn(&mut Rng, &PathCtx) -> PathOut,   // pieces, nodes, ports
}
```

**Context filter** for a segment: the walk gives the segment `(start, end, surrounding)`. The path type must satisfy:

| Segment | Allowed types |
|---|---|
| level, free air below | catwalk, skybridge, footbridge, catenary, zigzag, pier, gangway |
| level, over a solid roof or beam | beam-top, pipe-top, roof walk, ledge, conveyor |
| level, beside a façade or between buildings | façade gallery, arcade, alley, avenue, skew corridor |
| level, engulfable or reserved | bored tunnel, crawl duct, curved gallery (inside a rotunda) |
| height change up to 3 m | ramp (as part of any horizontal type) or a short stair |
| height change > 3 m | any PV type fitting the rise: flights up to the rise limit of `traversal.md`; helices and spirals for large rises; lifts above 32 m; ladders where the stratum allows |
| change of gravity | gravity gate (THR-09) |

- The legal set is intersected with the stratum's path weights (`regimes.md` §10); zero weights remove a type. **PH-01 and PV-01 are never removed**, so a legal choice always exists.
- Types that need a span (catenary, cable-stayed, zigzag) use two anchor surfaces taken from the surface list; if none fits, the type is skipped (the draw is still made) and the next is tried, down to the universal PH-01.
- **Modifiers MOD-01..09** are drawn independently after the type: width, enclosure (open, railed, caged, walled, glazed, tunnel), condition (intact, missing rails, a missing section with a detour: the detour is a second walk), light, ceiling height, rhythm, material, dressing, and angle (on-grid, off-grid by the local grain, free). A modifier that makes a path impassable (a missing section) must keep a detour as a second route, so connectivity is unchanged.

### 5W.6 Thresholds and hubs

*Owners: THR-01..09 (PW.3.13), HUB-01..11 (PW.3.15).*

- A **threshold** is placed on a route node where a route enters a structure, a sealed region, or a different gravity or atmosphere. Types THR-01..09 are drawn by context (a bored passage gets a bulkhead hatch; an engulfed route gets a portal frame or a broken wall; a gravity change gets a gate). The threshold never blocks: its opening is always at least the port minimum (`ports.md` §2).
- A **hub** (HUB-01..11) is the knot's structure in the L2 cell that contains the knot, and one more in each L2 cell on a `Spine` station or a `Road` crossing with chance 1/4. A hub is a special form whose family is the hub family of the character; it replaces step 3's first attempt and is placed even if `f` is met.

### 5W.7 Skin coverage

*Owners: TIS-05 (with the dead-end rule), HOR-02 (with PW.2.4).*

L1 gives the crust's **skin** surfaces (`§4W.6`): shells with openings wider than 32 m. Within an L2 cell the skin surface `S` is covered to at least **90 %** of its area:

1. Divide `S` into 32 × 32 m tiles. A tile is covered if the union of `Built` and `Engulfed` slots, plus cladding, lies within 1 m of the skin plane over at least 3/4 of its area.
2. Where a tile is not covered by a structure that grew there, add **cladding**: a thin slab (2–16 m deep) as a `Mass` or `Misc` piece of material `CONCRETE`, with windows from the cell hash (so that far window lights match the near ones, `the-web.md` §11.2). L1 openings stay open.
3. L2 may add **relief** (≤ 16 m deep) and openings narrower than 32 m only. Nothing larger.
4. The test measures the covered fraction over the whole skin of a crust in a 5 km region and requires ≥ 90 %.

### 5W.8 Places: the set pieces

L2 set pieces (the list of `the-web.md` §6.5): atrium with galleries, light well, cistern, organ room, window wall, observation pod, hanging garden, niche / shrine, generator room, abandoned dwellings, interchange, lift lobby, tram stop, valley balcony, grain seam plaza, tether foot. Each has a footprint, a facing, and a payoff weight; they are owned by PW.4+ and PW.7, and the dead-end pass only needs the footprint and the id. Hubs, set pieces and thresholds are all placed with the same grow acceptance test.

### 5W.9 ID tagging (census)

- Every piece, port, zone and route segment that comes from a registry idea carries a `Tag(u16)`, the registry index of its ID (a generated table `crates/gen/src/ids.rs` from `docs/proposals/registry.md`). A primitive may carry one primary tag; the others are in the instance list.
- `CellMeta` holds `instances: Vec<Instance>`, with `Instance { tag: Tag, key: u64 }`, where `key` is the hash of the feature's own key (so a feature that is clipped into many cells counts once in the census: the census counts distinct `(tag, key)`).
- Tags are part of the golden hash.
- The census (`web-report --census`, PW.2.11) counts instances per ID over the sample region, and reports IDs with zero.

### 5W.10 Salts and draw order

New L2 salts (block `0x0300–0x03FF`, append-only):

| Salt | Value | Key | Use |
|---|---|---|---|
| `TISSUE` | `0x0310` | cell (L2) | the four steps (§5W.2) |
| `TISSUE_FRAME` | `0x0311` | cell (L2) | frame of a structure in a grain |
| `SET_PIECE` | `0x0312` | cell (L2) | set-piece draws |
| `SKIN_WINDOWS` | `0x0313` | skin tile key | window lights (shared with the far band) |

Draw order in an L2 cell: reserve (no draws), route (anchors, walks, segments, loops, engulf marks), grow (attempts in order), dress (edges, skin, dead ends, closing loops). Every attempt and every pass draws a fixed number of values.

### 5W.11 As implemented: routes (PW.3.1, `worldgen::tissue`)

- `TissueCache` holds a `WebParent` per L1 cell: the skeleton and every L1 piece with its tag (inhabited beams left out). `tissue_routes(seed, cache, cell)` returns the `Tissue`: lattice, anchors, route slots, segments.
- **Lattice.** Slot centres are `cell centre + frame · ((s + ½) · pitch − 256)`, in `f64`; a slot is present when its centre lies in `[0, 512)³`. The pitch is 32 m scaled by the stratum's `scale` (§10.1 of `regimes.md`), a whole number of 16 m. The stratum is read at the cell centre with `character_near`.
- **Reserve.** A slot whose centre lies in a parent piece is `Solid`, or `Void` when the piece is the keep-out of a void and no solid piece covers it. Both can be crossed: a `Solid` slot costs 8 (the route is bored), a `Void` slot 2 (a span).
- **Anchors**, in order: artery ports and links by face (−x, +x, −y, +y, −z, +z) then type, each at the face centre of the cell and snapped to the nearest present slot; the knot at the cell centre; then 0–2 extras on free slots next to voids (any free slot when none).
- **Walks.** A weighted random walk (axis weight `255 − weave`, face-diagonal `weave / 2`, ×4 towards the goal, ÷8 into a `Solid` slot) with a budget of `4 · distance + 8`, then Dijkstra over every present slot (ties by lowest index). A diagonal move records its supporting slot, so every walk is a 6-connected path. The first anchor starts the route; each next one walks to the nearest route slot; 0–2 loops join route slots at least 4 apart.
- **Segments** are cut at corners and where the class of slot changes (free, void, solid); a segment is `Bored`, `Span`, `Rise` or `Level`, and is *engulfable* with chance 1/8 (the draw is always made) unless bored or spanning.
- The draws come from `hash(seed, L2, cell, TISSUE)` in the order: extras, route walks, loops, segment marks.

### 5W.12 As implemented: growth, supports and the skin (PW.3.2)

- **Pitch.** The lattice pitch is always 32 m. The stratum's scale (SC-05) multiplies the sizes of forms (and later of paths and the L3 module), not the lattice: a route slot of 96 m took 40 % of a coarse cell. This replaces the sentence of §5W.1 that scales the pitch.
- **Fill.** The target is `present · field / 255` slots. Parent solid counts as built (a skin plate and a beam are built matter), as do structures and swallowed routes. Engulfable chance of a segment is `1/8 + 5/8 · field`: routes run through buildings in dense strata.
- **Attempts.** `tissue_grow::grow` makes 64 attempts of 16 words each (all words are drawn even after growth stopped); it stops at the target or after 16 rejections in a row. An attempt reads the support from word 0 (weights of the character, zero for supports with no place to start, universal ones when the stratum bans them all), the form family from 1, the member from 2, up to two mutations from 3–5 (PW.3.8 applies them), the size from 6–8, the place from 9–11, the entrances from 13–14.
- **Place.** A support yields a box: Standing and Hanging grow it from an anchor slot by whole free layers (x, z and up or down) within the form's size, six anchors tried; Suspended starts 1–3 slots below a ceiling and carries cables; Spanning fills a gap of 2–14 slots between two supports; Sideways lays the form on its side from a wall; Collar is a ring of four boxes round a vertical solid; Embedded takes a 2×2×2 inside solid; Piled is 3–6 small boxes touching each other; Nested fills the void of a hall from its floor; Colliding is a box in a frame turned 45° from the cell's. Radial waits for wells.
- **Accept.** All slots present and free (route slots only of engulfable segments and not anchors; solid, void or built only where the support means to overlap), and the structure touches a support: solid, built, route, or the cell shell inside a crust (the shell is the lattice boundary when the slot beside it lies in a crust). If it does not fit, the largest dimension shrinks and it is placed again, down to one slot.
- **Entrances.** One per 30 m of footprint, 1 to 8, on route slots next to the structure first; a new access walk only for the first entrance (Dijkstra over every slot a structure has not taken) or when it is at most 4 slots. A cell off the skeleton starts its route at the first entrance. A structure with no entrance is rejected.
- **Skin.** `skin_pass` takes the L1 skin plate pieces that meet the cell, finds the outside as the side that is not crust, and cuts the plate into 32 m tiles (in plate coordinates, so tilted plates tile in their frame). A tile whose centre is in the cell is covered when 12 of its 16 probes (0.5 m outside) lie in a structure; the rest are merged into runs and clad with 2–16 m slabs of concrete.
- Cost: about 40 ms per cell (routes 7 ms). The census for SUP-01..10 and for the per-support counts is `tissue_grow` test `every_support_occurs`.

### 5W.13 Support variants (PW.3.3, PW.3.4, `worldgen::tissue_variants`)

The registry rows SV-01..26 are a garbled list of the variants of `the-web.md` §6.2. The mapping used here, support by support, with the plain support as the first entry:

| Support | Variants |
|---|---|
| Standing | plain · SV-01 on stilts over a route · SV-02 on a slope with stepped footings |
| Hanging | SV-26 free (the plain hanging) · SV-03 kissing a standing tower below, joined by a stair · SV-04 ending in a viewing pod · SV-05 chandelier cluster · SV-06 broken off with floors exposed · SV-07 hung from cables · SV-08 swung off-plumb |
| Spanning | plain bridge-building · SV-09 pillar building · SV-10 arch building with a route under it · SV-11 fallen tower lying diagonally across a void |
| Sideways | plain horizontal skyscraper · SV-12 from both sides of a trunk · SV-13 sagging floor by floor · SV-14 jutting out at a slant · SV-25 300 m long, stepped |
| Collar | plain ring · SV-15 half ring · SV-16 true helix climbing the trunk |
| Embedded | plain cliff dwellings · SV-17 a façade with nothing behind it · SV-18 windows only |
| Suspended | plain gondola · SV-19 cable-hung platform town · SV-20 cages |
| Piled | plain jenga · SV-21 omnidirectional cluster · SV-22 leaning pile across an alley |
| Nested | plain building in an atrium · SV-23 a town in a hall · SV-24 a district inside a hollow tether |
| Colliding, Radial | plain only |

- **Draw.** Word 10 picks the variant: three in four are plain, the rest share the variants of the support equally. A variant that has no place in the cell is replaced by the plain support (the attempt is not lost).
- **Pieces.** Besides boxes, a structure may hold shaped pieces whose boxes count as taken slots (`Shaped::Cyl`, `Stair`, `Cage`, `Helix`, `Facade`), cables (thin vertical pieces from a hanging box to the cell top), and windows on a face with no mass behind them (SV-18; such a structure takes no slot and needs no entrance). Oriented parts (SV-08, SV-11, SV-14) are boxes in `Frame::table(yaw, tilt)` rasterised by slot centre.
- **Geometry.** Stilts are four pillars from the route to the box; footings are columns of 1–3 slots below a box that sits 1–2 slots up; kissing needs a built top 2–4 slots below the hanging box and puts a stair run in the gap; the pod is a cylinder under the middle of the box; a chandelier is three to six pendants of different length under one ceiling; a broken structure is cut diagonally along its width; the pillar and arch buildings stand on the ends of a gap between two supports (the arch's legs are two slots high); the helix is a faceted stair round the trunk (at most 256 treads) in a 3×3 box whose centre is the trunk; a façade is a 4 m slab on the host side of its box.

### 5W.14 Forms (PW.3.5 to PW.3.7, `worldgen::forms`)

A form is `fn(size: [w, h, d] in slots, rng) -> Option<FormOut>`; `FormOut` holds boxes, shaped pieces and boxes turned in `Frame::table(yaw, tilt)` about the centre of their base, all in slot units from the corner of the bounding size. `None` means the form does not fit that size, and the slab tower stands in (the structure records the form actually used). The structure takes the whole bounding box as **envelope** (its slots count as built whatever the pieces leave open), because L2 draws envelopes and L3 refines the inside.

Size is drawn per form in metres (`[width, depth, height]`), multiplied by the stratum's scale and raised in slots; in a dense stratum the unit draw is raised to `1 / (1 + 4 · field)` so that structures are large.

| Form | Family | Pieces |
|---|---|---|
| FORM-01 slab tower | TOWER | one box |
| FORM-02 ziggurat | STEP | 2–4 tiers, each inset by one slot per side, widest at the bottom or (inverted) at the top |
| FORM-03 needle | TOWER | a box 1–2 slots across |
| FORM-04 tube tower | TOWER | four walls round an open core |
| FORM-05 H-pair | TOWER | two towers and 2–3 bridges |
| FORM-06 comb | TOWER | a base slab with a tooth every second slot |
| FORM-07 cage | FRAMEWORK | the edges of the box and a floor every 8 m |
| FORM-08 cluster | TOWER | 5–30 boxes, each touching an earlier one |
| FORM-09 drum or silo | ROUND | a cylinder, upright or lying along x or z |
| FORM-10 terrace cliff | STEP | terraces rising along x |
| FORM-11 gantry | FRAMEWORK | two legs and a beam |
| FORM-12 machine | PLANT | a lying turbine, a stack of cylinders, a bank of pistons or radiator fins |
| FORM-13 rack canyon | FRAMEWORK | two long walls with a canyon between |
| FORM-14 tank farm | PLANT | cylinders of 2×2 slots on a base slab |
| FORM-15 ruin | RUIN | columns of ragged height (60 % of the footprint) |
| FORM-16 rotunda | ROUND | a ring wall (16 facets) round a central drum |
| FORM-17 twisted tower | ANGLED | one turned box per floor, each turned 2–4 headings more |
| FORM-18 leaning tower | ANGLED | a box turned about its base by a tilt of 1–3 |
| FORM-19 dome | ROUND | four rings of 16 facets narrowing upwards |
| FORM-20 polygonal ring | ROUND | a wall of 8 or 16 facets |
| FORM-21 starburst hub | ANGLED | a core and eight arms every 45° |
| FORM-22 wedge | ANGLED | a prism whose roof rises towards one of four directions |
| FORM-24 roofed ring | ROUND | a FORM-20 or FORM-16 ring wall with a roof, by a draw keyed by where the ring is: two in five roofed, half of those flat (a 1 m disc on the wall) and half with an oculus (a 1 m ring of facets from 0.3 of the radius out to the wall); three in five stay open-topped |
| FORM-23 cone or funnel | ANGLED | eight stacked cylinders narrowing upwards, or downwards |

The support variants (§5W.13) work on the form's boxes (its envelope when it is made of shaped pieces).

### 5W.15 Mutations (PW.3.8, `worldgen::mutations`)

Groups as `regimes.md` §9: POSE 01 inverted, 02 tilted, 03 twisted · DAMAGE 04 broken, 05 overgrown, 15 swallowed · COPY 06 doubled or mirrored, 07 stretched · HOLLOW 08 hollowed, 09 caged, 10 pierced · LIGHT 11 dark, 12 lit · SCALE 13 scaled, 14 fused. An attempt has none (5 in 10), one (3 in 10) or two (2 in 10) mutations; the group is drawn by the stratum's mutation weights, the member uniformly. Each acts on the form's output before placement: inverted flips y; tilted and twisted turn the pieces about a common pivot (`Turned.pivot`); broken ragged-cuts above 60 % of the height; overgrown hangs vines; mirrored keeps the left half and reflects it; stretched builds at half length on one axis and doubles it; hollowed leaves four walls; caged turns boxes into frames; pierced cuts a one-slot passage whose ends become entrances when a route reaches them; dark uses the metal material, lit adds windows on the sides; scaled makes the size a quarter (1 in 4) or twice; fused widens the box one slice over a built neighbour and allows the overlap; swallowed puts an older form, half size, inside in metal.

### 5W.16 Horizontal paths (PW.3.9, `worldgen::paths`)

PH-01..10 (PW.3.9) and PH-11..20 (PW.3.10) for level, span, bored and swallowed segments; PW.3.11 takes rises. Draws come from `hash(seed, L2, cell, PATHS)`: seven words per segment, always, so skipping a segment never moves the rest. Word 0 picks the type, 1 the width, 2 the rails (none, one, both), 3 windowed or blind, 4 the side of a façade, 5 and 6 sizes (sag, amplitude, pipe radius, ceiling).

- **Legs.** A segment is cut into straight legs of equal height. A leg starts and ends at slot centres, overlaps its neighbour by half a width at corners, and reaches the cell's side when its end slot is on the lattice edge. The walking surface is the floor of the route's slots, so one deck height holds for every type.
- **Context** (from the slots next to the leg): *roof* when at least half the slots below are solid, built or engulfed; *wall* on a side when at least half the lateral neighbours are; *open above* when every slot above is free or void; *steps* is the slot count minus one. Legal types: PH-01 always; open below: PH-02, PH-05 (2+ steps), PH-04 (2+), PH-03 (2+ and open above); over a roof: PH-06, PH-07; with a wall: PH-08, PH-09, and PH-10 when both sides have one (2+). The two anchors of a span type are the leg's ends (they are route slots, so they are anchored by construction).
- **Weights** are the stratum's `paths[WALK]` for PH-01, 03..08 and `paths[ENCLOSED]` for PH-02, 09, 10; PH-01 is never below 1.
- **Types.** PH-01 grating deck, 1.2–3 m, ribs every 4 m, rails by draw. PH-02 enclosed bridge 3–8 m wide, 3.2 m high, blind or with a glass band. PH-03 deck 2–5 m with a pylon at mid span and stay cables (stepped boxes, no collision). PH-04 deck that sags (3–12 m, under 27°) built from ramps with an overlapped underside, handrail cables. PH-05 nine-turn bridge: runs of 8–14 m alternating ±2–4 m sideways, jogs one width long, the end runs on the centre line. PH-06 tread inlaid in the top of a beam, kerbs and lamp posts. PH-07 pipe set into the roof with a 0.9 m tread on top. PH-08 ledge 1.2–2 m with ties to the façade 15.5 m off. PH-09 gallery 3–6 m: roof, wall one side, posts and rail on the other. PH-10 arcade 4–10 m: pillars and lintels every 8 m, 4–6 m ceiling.
- **Tests.** `every_type_is_walkable` probes a floor 5 cm under, and nothing solid at 0.25, 0.9, 1.5 and 1.85 m above, every metre of each type's centre line (`walk_line`) in six variants and two axes; `pieces_stay_in_the_cell`; `catwalk_is_always_legal`; `every_path_type_occurs` (2 000 grown cells: all ten occur; PH-10 is rare, 23 cells).
- Not in `l2_cell` yet (PW.3.21). Screenshots waived.

**PH-11..20 (PW.3.10).** Context adds: *bored* (the segment runs through parent solid) and *swallowed* (any slot is inside a structure) get PH-01, PH-11, PH-12, and PH-20 when a swallowing structure has a ring wall; open segments gain PH-20 when a rotunda (a structure with a `Ring` piece) is next to them (a swallowed route in a rotunda is too rare to occur otherwise: 0 in 600 cells); over a roof PH-15 and PH-18; in the open PH-16 and (with air above) PH-17, both from 2 steps; beside a wall PH-13, and PH-19 from 2 steps; between two walls PH-14. Families: ENCLOSED has 11, 12, 20; STREET (`paths[2]`) 13, 14, 15, 19; WALK 16, 17, 18.
- PH-11 lined passage 3–6 m wide and 3.2 m clear, ribs and a lamp row. PH-12 crawl duct 1.2–1.6 m wide and 1.3 m clear. PH-13 avenue 10–28 m wide with kerbs and lamp posts every 24 m. PH-14 alley 2–6 m wide with walls 14 m high (doors every 18 m on one side), a pipe, beams overhead. PH-15 roof tiles of 8 m, each 0, 0.15 or 0.3 m high (steps at most 0.3 m), parapets and vents. PH-16 pier 3–6 m on piles 24 m deep, bollards, a lamp at the head. PH-17 gangway 1.2–2.4 m on chains from a girder 14 m up. PH-18 conveyor 1.2–2 m, belt set in the roof, cleats 4 cm high every 2 m. PH-19 corridor that jogs 4–12 m sideways at 45° and back, the diagonals built from overlapping squares (the generator has no composed frames, so a true skew frame waits for a use that needs one). PH-20 gallery bowing up to 10 m (a fifth of the leg) in a parabola, built from overlapping squares, with an inner colonnade and a roof-height cut.
- **Bores.** Every leg of a bored or swallowed segment adds a clear box to `PathPieces::bores` (PH-11, 12, 20 their own, others a default `width + 2` by 4 m). They are air to cut out of the parent's solid where the route passes: not drawn and not collidable. **PW.3.21 must apply them** (clip the parent pieces, and the structures' envelopes, against them) or the passages stay blocked; until then no bore has any effect.

### 5W.17 Vertical paths, first half (PW.3.11, `worldgen::paths_v`)

A rise segment is a straight run of `k` slots, so it climbs `32 k` m inside one 32 m column (the lattice pitch is always 32 m). At the walking stair of `traversal.md` (0.1875 m rise, 0.3125 m tread, 31°) one slot needs 171 steps and 53 m of run, so nothing climbs a slot in a straight line: every type folds, spirals or winds. The plan is drawn in `(u, v)` about the column centre and mapped to the lattice frame by a swap and two flips (draws). The route enters at the centre of the column at the foot and leaves at the centre at the top; both ends are decks. Ids in `PathPieces::chosen` are `20 + PV number`; the draws are the same seven words per segment as §5W.16.

- **Folded stairs** (PV-01, 02, 03, 07, 08, 09, 10): flights alternate between two lanes at `v = ∓w` (lane width `w`) along `u`, joined by landings that span both lanes; a corridor `w` wide down the middle `v = 0` is kept free and carries the foot deck (at the bottom) and the top deck. Flights are as equal as possible: a short last flight would put its landing over the flight before it. Flight length is at most `2 · lh`; the footprint stays inside ±16 m.
  - PV-01 straight flight: `lh = 11.5`, width 2–4 m. PV-02 switchback: `lh = 5–8`, width 2–3.5 m (about 3× the flights). PV-03 scissor: PV-01 plus a second stair turned half a circle in lanes at `v = ±3w` (the second stair is not on the route). PV-07 fire escape: 1–1.2 m wide, `lh = 3.5`, ties to the façade (a wall beside the column) at each landing. PV-08 cascade: 7–9 m wide, `lh = 6`, only from 3 slots (96 m). PV-09 stair in a wall's thickness: 2–2.5 m wide, walls 1 m thick either side; also the choice for bored and swallowed runs. PV-10 grand stair: 9–10 m wide.
- **PV-04 square spiral:** a ring of side 24 m round an open well (the centre is the route, so a pillar is not possible; the registry's "pillar or well" is the well), flights along the four sides counter-clockwise, corner landings; the foot deck runs from the centre to the first corner, the top deck from the last landing inward and along the middle to the centre.
- **PV-06 helical ramp:** the same ring with ramps at 0.268 (15°), width 4–6 m.
- **PV-05 true helical stair:** `orient::helix_stair`, 32 treads a turn, inner radius 4 m, width 2.5–4 m, rise 4 Q (so the climb is exact), up to 4 slots; built in table frames of yaw only, so it needs a lattice frame without tilt (otherwise PV-01 is built).
- **Legal types.** Open: PV-01, 02, 03, 04, 06, 10; PV-05 with a flat lattice frame and up to 4 slots; PV-07 and PV-09 with a wall beside the column; PV-08 from 3 slots. Bored or swallowed: PV-01, 09, 04, and PV-05 as above. Weights are `paths[STAIR]`; PV-01 is never below 1.
- **Deviations (decide):** PV-10 is 18–20 m wide in total (two 9–10 m lanes), not 60–150 m, because a column is 32 m; wider needs lateral slots. PV-03's second stair is not connected to the route.
- **Bores.** Bored and swallowed runs add the box the stair fills (plus 0.5 m) to `PathPieces::bores`.
- **Tests.** `every_type_climbs_the_column`: all ten, at 1, 2 and 4 slots, four plan orientations; the route starts at the centre of the foot and ends at the centre of the top, has a floor under every sample and 1.85 m of headroom above, and the pieces stay inside the 32 m column. In the 2 000-cell run all ten occur (PV-08: 255, PV-04: 5 275). Only the identity frame is probed; tilted frames use the same lattice-frame code (PV-05 excepted).
- **Plan against the legs at the foot (PW.3.12b, version 12).** The first lane stands solid from the floor up on plan −v, so a leg that meets the foot from that side runs into it, and a leg across the middle runs into the lanes. `place_rise` therefore draws the plan (swap and flips) as before and, if a horizontal segment leaves the foot slot on the solid side, takes the first of the eight plans that has none; PV-03 is solid on both sides and falls back to PV-01 when no plan is free. Measured on the walk test (52 000 samples on open level segments): samples blocked by stairs and ramps 423 to 16.
- Not in `l2_cell` yet (PW.3.21). Screenshots waived.


### 5W.18 Vertical paths, second half (PW.3.12, `worldgen::paths_v`)

PV-11..19, ids 31..39, in the LIFTSLOPE family (`paths[4]`). They reuse the machinery of §5W.17; a flight now has a style (rise and run of a unit): the walking stair (3 Q by 5 Q), the stair beam's stair (4 Q by 8 Q, a slope of 1 in 2) and the ramp of the inclined plane (25°, half-metre units).
- **PV-11 stair-bridge:** PV-01's folded flights with `lh = 10.5`, width 3–4.5 m, and four piers under every landing, each down to the landing two flights below (or the foot).
- **PV-12 stair beam:** folded flights in the beam stair (slope 1 in 2), `lh = 12.5`, 2–3 m wide; the flight is a solid wedge, so it reads as a beam. It is not tied to an actual L1 stair beam (those are skew; the lattice route cannot follow them): **deviation, decide.**
- **PV-13 wrap round a trunk:** PV-04's square spiral on a ring of side 16 m, with a trunk 3 m clear of the flights from 3 m above the foot to 3 m below the top, so the centre stays walkable at both ends. (This also gives the registry's "pillar" of PV-04.)
- **PV-14 terraces:** folded flights 8–10 m wide, `lh = 5`, landings 6 m deep.
- **PV-15 inclined plane:** folded ramps at 25°, 5–9 m wide, `lh = 11`, landings 4 m deep (not a kilometre long: a column is 32 m).
- **PV-16 lift:** a narrow stair (so the route is walkable without the car) and a shaft 6 m square in the plan corner `(+u, +v)` with a door 3 m by 4 m at the foot and at the top; variant by draw: freight (concrete walls), cage (grating walls), open platform (four posts). In identity frames a `LiftShaft` volume marks the car's way for the engine. **The car, gates and rides are runtime** (`lifts.md` §10, P3.4.2b); the generator only draws the shaft and the way in. Only from 2 slots (64 m).
- **PV-17 paternoster:** two open shafts side by side with a loop of cabins (no collision, every 6 m, the second shaft half a spacing off), a platform at each stop, and the same narrow stair. From 2 slots.
- **PV-18 escalator:** a dead escalator: a narrow stair (1.4–1.6 m) with a stepped handrail on each edge.
- **PV-19 funicular:** a stair beam with a rail on each lane edge, 15 cm above its surface.
- **Legal types.** Open: PV-11, 12, 13, 14, 15, 18, 19; PV-16 and 17 from 64 m. Bored or swallowed: PV-12 as well as §5W.17's. Weights are `paths[LIFTSLOPE]` (a zero removes them; PV-01 remains).
- **Tests.** `every_type_climbs_the_column` now covers all 19 (foot to top at the centre, floor under every sample, 1.85 m of headroom, inside the column); all 19 occur in 2 000 cells.
- Not in `l2_cell` yet (PW.3.21). Screenshots waived.


### 5W.19 Thresholds (PW.3.13, `worldgen::thresholds`)

A threshold goes on every structure entrance (`Structure::entrances`) and at both ends of every bored passage (the face between a route slot outside the solid and the first solid slot of a `Bored` segment), in the frame of the face: `s` into the structure, `t` across, `y` up from the route slot's floor. Draws come from `hash(seed, L2, cell, THRESHOLDS)`, three words per face (type, size, spare), faces in order: structures, then bored segments. The opening is at least 2 m wide (the port minimum) and 2.4 m high over its depth, and a sill is at most 0.4 m. Each threshold adds a clear box to `Thresholds::bores`, to be cut from the structure or the solid (**PW.3.21 must apply them**, as it must the path bores).
- **Legal types.** A horizontal structure entrance: THR-01 (weight 2), 02 (3), 03 (1), 04 (2), 06 (1), 07 (1), 08 (2). An engulfed route: THR-02 or THR-04 (3 each), as the spec says. A bored end: THR-01 (3) or THR-06 (1). A hole down into a structure: THR-05. A hatch up into one: THR-01 as a ceiling collar.
- **Types.** THR-01 bulkhead hatch: a thick metal frame, 2 m clear, with a 0.3 m coaming. THR-02 portal frame: concrete, 3 m clear, 4 m high. THR-03 giant gate: 8–12 m wide and 11–14 m high, shut, with a 2.2 m by 2.6 m door. THR-04 broken wall: stumps of three heights and rubble beside a 3 m gap. THR-05 floor hole: a collar on three sides and a rubble stair of thirteen 0.3 m steps down. THR-06 airlock: two hatches 6 m apart with a chamber between. THR-07 window: a 0.4 m sill, 2.4 m clear, a glass transom. THR-08 threshold of light: a glowing frame, a lit floor strip and a ceiling panel beyond.
- **THR-09** (gravity gate) waits for the gravity module: **PW.3.13b**.
- **Count.** About 120 thresholds a cell on average (800 cells), which is a few hundred primitives a cell and as many clear boxes; if that is too much, the entrances are the lever (most are L3 doors).
- **Tests.** `openings_meet_the_port_minimum` (all but THR-05, both axes, both directions, three sizes: nothing solid in the 1.9 m wide, 2.35 m high opening, sills under 0.45 m), `the_rubble_stair_is_walkable`, `every_threshold_occurs` (800 cells, THR-01..08).
- Not in `l2_cell` yet (PW.3.21). Screenshots waived.


### 5W.20 Path modifiers (PW.3.14, `worldgen::modifiers`)

Eight words per segment from `hash(seed, L2, cell, MODS)`, always, drawn after the type is known and built on every horizontal leg (and, for the material, on every rise). A modifier never narrows the walkway below the type's width, and never puts anything solid within 2.2 m above the centre line, so walkability is unchanged. `PathPieces::mods` records them per chosen segment.
- **MOD-01 width:** the type's own range (§5W.16, §5W.17), drawn per segment; recorded as `Params::width`.
- **MOD-02 enclosure** (open 4/10, railed 2, caged 1, walled 1, glazed 1, tunnel 1): only on open flat decks (PH-01, 03, 06, 07, 16, 17, 18). Railed: rails 1.1 m on both edges. Caged: rails, plus bars every 4 m up to a top bar at 2.6 m. Walled: walls 0.3 m thick and 2.6 m high, open above. Glazed: a 0.9 m wall and grating panes up to the roof. Tunnel: solid walls to the roof.
- **MOD-03 condition:** intact, or missing rails: on a railed or caged enclosure (1 in 4) the rails stand in alternate 6 m stretches. (The spec text also names a missing section with a detour; the registry row does not, so it is not built.)
- **MOD-04 light:** dark, or a lamp row on the façade side every 8 m (no collision).
- **MOD-05 ceiling** under a glazed or tunnel roof: crushing 2.2 m, normal 3 m, cathedral 6 m (drawn evenly); open without a roof.
- **MOD-06 rhythm** (PH-01, 06, 07, 08, 13, 16, 17, 18): none (1/2), frames, arches (frames with stepped haunches) or ribs (thin frames every 3–5 m); frames every 8–16 m stand outside the deck at `±(w/2 + 0.7)` and the lintel is at 3 m plus up to 5 m on wide decks, or above the roof.
- **MOD-07 material:** structural pieces (concrete and metal) become concrete, metal or warm, weighted by the stratum's first three material weights plus one. The material slots proper are defined in PW.4.x and will replace this.
- **MOD-08 dressing** (not on the enclosed types PH-02, 09, 10, 11, 12): none, three cables, two pipes, or emissive signage glyphs every 14 m, all overhead or on the edge.
- **MOD-09 angle:** on-grid, off-grid, free. It turns the lintels and ribs of a rhythm about the vertical (off-grid 2, 4 or 6 headings, free 3 to 11) when the lattice frame has no tilt; posts stay on the grid. The generator has no composed frames, so the deck itself cannot skew (PH-19 and PH-20 are the skewed types).
- **Tests.** `modifiers_keep_every_type_walkable` (all twenty horizontal types, forty modifier draws each, both axes: floor under and 1.85 m clear above the walking line), `every_modifier_value_occurs` (all values; ineligible types never get an enclosure, rhythm or dressing), and the 2 000-cell census: every value of every modifier occurs.
- Not in `l2_cell` yet (PW.3.21). Screenshots waived.


### 5W.21 Hubs (PW.3.15, `worldgen::hubs`)

Hubs are forms of their own (`HUBS`, `Structure::form = 100 + n`), built by the same placement as the forms and placed at the start of step 3 whatever the fill; `Tissue::hubs` lists `(HUB id, structure index)` for those that fit as themselves (a hub, a set piece or a local place whose builder cannot make the size it was fitted to is not placed: there is no slab fallback for landmarks, PW.3.15c; a plain form still falls back to the slab tower). The draws are always made: two sets of sixteen words, three more for the knot's retries, a pick and a chance.
- **Retries and the least size (PW.3.15c, version 16).** The knot's hub is tried up to four times: with the drawn words and kind, then with each of three sets of fresh words and, where its family has several kinds, the next kind in the family's order (the drawn kind first). A contextual hub is tried once. `fit` shrinks a hub's size only down to the low end of its range (scaled by the stratum, in slots), so a builder is never asked for less than it can make.
- **The knot's hub.** The cell that holds the knot (`Skeleton::knot_hub`, from `l1_frame::knot`) gets one. Family 0 junction hall: HUB-01 (2) or 02 (1); 1 lift nexus: HUB-04; 2 machine: HUB-03 (2), 05 (1) or 06 (1); 3 gravity furnace: HUB-10; 4 sanctuary site: HUB-11.
- **A contextual hub**, with chance 1/4, the first that applies: HUB-08 tram terminus where a tram tube port is anchored in the cell; HUB-07 valley rim beside a void; HUB-09 Between overlook where a crust meets a Between. **World features (PW.3.15b, version 29):** before those, HUB-04 (a stair cathedral) in the L2 cell where a Spine's column meets a crust (the middle two of eight in x and z, at the station's row), and HUB-01 (a junction hall) in the L2 cell of a Road crossing (a cell with a road along x and one along z, at the higher deck's row). Over a sample, 6 of 14 stations held their hub (the chance is 1/4 and the fit may fail); 2 crossings were found and neither held one.
- **Forms** (all in slots, sizes 128–512 m): 01 low hall with a tower and a beacon cone, lit; 02 floor, roof and a grid of columns; 03 horizontal drum between two aisles with fins; 04 ring wall round a helical stair with four corner towers; 05 roofed floor with a row of tanks; 06 two racks with a floor and bridges, lit; 07 three terraces stepping to an edge; 08 platform under a roof on columns with an end block, lit; 09 column with a platform thrust out at the top and a pod; 10 drum, dome and chimney; 11 platform with a ring wall and a pillar, lit.
- **Measured.** Before PW.3.15c 434 of 600 knot cells got a hub (the rest fell back to a slab); after, 585 of 600 get the knot's own hub (97.5 %), and none is a slab; per hub over the sample: 01: 137, 02: 90, 03: 64, 04: 129, 05: 54, 06: 33, 07: 1, 08: 31, 09: 66, 10: 31, 11: 47 (HUB-07 is now rare: 1 in 600 knot cells and 800 other cells).
- **Tests.** `every_hub_builds_inside_its_size` (each at four sizes, pieces inside), `every_hub_occurs` (knot cells of 600 L1 cells plus 800 other cells: all eleven).
- Not in `l2_cell` yet (PW.3.21). Screenshots waived.


### 5W.22 Set pieces (PW.3.16, PW.3.17, `worldgen::sets`)

Forms of their own (`SETS`, `Structure::form = 200 + n`), placed in step 3 after the hubs and before the generic growth, so the fill counts them. Per cell: none (1/2), one (1/4) or two (1/4), each kind drawn by weight from what the cell's context allows; the draws are always made. `Tissue::sets` lists `(SET id, structure index)` for those that fit as themselves (a set piece whose builder cannot make the size it was fitted to falls back to a slab, as hubs do: PW.3.15c).
- **Context.** Any cell: SET-03 (2), 05 (2), 07 (1), 08 (3), 10 (2). Fill at least 0.5: SET-01 (3), 02 (2), 04 (2), 09 (2). A free slot beside a void: SET-06 (3), 14 (3). Four or more anchors: SET-11 (3). A lift trunk port: SET-12 (4). A tram port: SET-13 (4). A grain seam (a neighbouring L2 cell has another frame, `Tissue::seam`): SET-15 (4). A tether landing in the cell (`Tissue::tether_foot`, the foot of an STR-03 piece): SET-16 (4).
- **Forms** (slots; sizes 64–384 m): 01 atrium: four walls, a floor and a gallery every 3 slots; 02 light well: a tall ring wall on a base, lit; 03 cistern: a broad low tank on a slab; 04 organ room: a roofed hall with a back wall and a row of pipes of rising height; 05 window wall: one thin wall with buttresses, lit; 06 observation pod: stalk, arm and a pod, lit; 07 hanging garden: platforms hung under a beam with vines; 08 shrine: an alcove with a statue, lit; 09 generator room: a roofed hall with turbines lying along it; 10 dwellings: little boxes piled in a grid, lit; 11 interchange: four decks at four heights on four posts; 12 lift lobby: a hall with a square shaft; 13 tram stop: a platform under a canopy on posts; 14 valley balcony: a block with a slab thrust out and posts; 15 seam plaza: a triangular slab; 16 tether foot: a slab with a collar.
- **Deviations (decide).** The set pieces are placed where the structures can stand, not at the thing that gave them their context: SET-16 is not at the tether's foot, SET-15 not on the seam, SET-06 and 14 not at the void's edge (only in a cell that has one), and SET-11's four decks are not joined to actual routes. SET-12's shaft has no lift (the car is runtime).
- **Measured** over 3 000 cells: per type 98, 80, 205, 71, 219, 1, 98, 343, 79, 269, 57, 13, 23, 6, 233, 26 (SET-06 and 14 are rare because valleys are).
- **Tests.** `every_set_piece_builds_inside_its_size` (five sizes, pieces inside), `every_set_piece_occurs` (3 000 cells: all sixteen; about a minute).
- Not in `l2_cell` yet (PW.3.21). Screenshots waived.


### 5W.23 Dead ends and legibility (PW.3.18, `worldgen::payoffs`; TIS-05, RULE-01, RULE-02)

Run after growth, before the skin and the paths. A **dead end** is a route slot with at most one route neighbour that is no anchor (port, artery link, knot or extra) and no structure's entrance. In route order, each draws four words and takes the first stage that applies:
1. a **set piece** (chance 1/3): the kinds of `sets::legal`, first by weight and then the next two, grown in the free slots beside the leaf (the box of the kind's smallest size, centred across the leaf), with an entrance at the leaf: a structure of the cell;
2. a **view**: a ray of seven free or void slots (224 m; the spec says 200 m, and a ray to the cell face does not count because the neighbour is unknown) along one of the four horizontal directions; a balcony 6 m wide to the face, with a rail along its free end;
3. a **lift** (chance 1/4): the nearest route slot of the same column at another height with only free slots between joins the leaf by a new rising segment, which `paths` builds as PV-16 (`Tissue::lift_segments`);
4. a **light**: a lamp on a post at the leaf, which always fits.
`payoffs::unpaid` is the validator (RULE-01): leaves without a payoff; the test requires none in 2 000 cells. **Legibility (RULE-02):** the cell that holds the knot has a beacon (a 24 m spire of the beacon material, visible far, in a corner of the knot's slot clear of the route); the lit hubs and set pieces and the arteries' bias towards the knot are the rest. **TIS-05** also names the edges: rails and thresholds are in §5W.16, §5W.19 and §5W.20.
Measured (2 000 cells): 101 dead ends: 25 set pieces, 46 views, 1 lift, 29 lights. Set pieces and local places now stay within the cell's fill (the fill test is at 76 % within ±0.10, mean error 0.064; the bar is 75 % and 0.07).

### 5W.24 Local places and near details (PW.3.19, `worldgen::locals`, `worldgen::modifiers`)

**Local places LOC-01..06** are forms like the set pieces (`Structure::form = 300 + n`, `Tissue::locals`), placed after them with chance 1/4 per cell: LOC-01 light well ringed by galleries (fill 0.5), LOC-02 atrium under a glazed roof (0.5), LOC-03 thin spot: a slab two slots thick with a great lit window, where a column of the lattice has one or two crust slots, LOC-04 cut-away (a building with its front wall missing, 0.25), LOC-05 courtyard sunk into a low block (0.25), LOC-06 two towers leaning towards each other (tilt 1, yaws 0 and 32; 0.375). Measured over 2 000 cells: 27, 40, 28, 41, 40, 40.
**Near details NEAR-01..05** are a ninth modifier word per path segment (none 1/2, then one of five): NEAR-01 a grille (a framed row of bars) beside the walkway; NEAR-02 a slit view (a thick wall with a gap 0.3 m wide); NEAR-03 a doorway framing a lamp at the far end of a long leg; NEAR-04 gaps 0.2 m wide across a catwalk's grating every 6 m (PH-01 only; narrower than a foot, so they show the drop and cost nothing); NEAR-05 windows (rows of lit panels along the façade side). Types that wander sideways (PH-05, 19, 20) take none of NEAR-01..03 and 05. The structures' own windows (the lit forms, the skin) are the same idea at larger scale. The modifier walkability test covers them.

### 5W.25 Where the ideas of free orientation are (PW.3.20, ORI-01..11)

| ID | Where |
|---|---|
| ORI-01 grain | `regimes.md` §7.3; frames per L2 cell (`WebFields::frame_l2`) |
| ORI-02 fallen towers as bridges | SV-11 (§5W.13) |
| ORI-03 tilted strata | `StackGeometry::Tilted` (`regimes.md` §8) |
| ORI-04 oblique struts | oblique beams, FRM-02 (§4W.2.1a) |
| ORI-05 stair beams and funiculars | FRM-11, PV-12, PV-19 (§5W.18) |
| ORI-06 rotundas, drums, domes | FORM-09, 16, 19 (§5W.14) |
| ORI-07 true helices | PV-05 (§5W.17), SV-16 |
| ORI-08 catenaries | PH-04 (§5W.16) |
| ORI-09 polygonal rings | FORM-20 |
| ORI-10 radial districts | **PW.3.20b** (needs the gravity wells, PW.6.1) |
| ORI-11 diagonal routes across a hall | PH-19 (§5W.16) |
`tests/orientation.rs` finds each one by census in the generated world (grain frames in 64 L0 cells, a tilted stack, oblique and stair beams in 400 L1 cells, and the forms, variant and path types in 1 500 cells).


### 5W.26 The Web L2 cell and the game (PW.3.21, `worldgen::l2_web`, version 11)

`l2_build` (and so `l2_cell`, `l2_cell_in`) now returns the Web tissue: structures (hubs, set pieces, local places and the dead-end set pieces among them) in placement order, the skin's cladding, path pieces, thresholds, payoffs. Ports are the L2 face ports of the L1 skeleton (`cell_ports_in`). The clear boxes of paths and thresholds are `KeepOut` constraints (air that L3 must leave open and that the parent's solid is to be cut for: PW.3.21b). There are no masses until PW.4. `GEN_VERSION` is 11.
- **Tram volumes.** A tram line's reserved volume is clear air: a slot it touches is absent (`tissue::drop_tram_slots`), and any piece that still meets it (a turned structure reaches past its slots) is dropped (`l2_web::drop_in_tram_volumes`). The tram test `no_l2_primitive_enters_the_envelope` holds.
- **Density.** After the first runs the spacing of repeated pieces was doubled (ribs, lamps, glyphs, piers, cleats, rhythm frames) and a structure gets thresholds on its first two entrances only.
- **Engine.** L3 is off (`meshpool::L3_ENABLED`) and structures collide as they are drawn; a chunk over the frame's upload budget uploads alone, up to four times the budget; the default start is a walkable one (`cellscene::knot_spawn`: the start of the longest level segment in the knot's L2 cell, 1 m above its floor).
- **Not walkable yet (stated so it is not mistaken for a bug).** Bored tunnels, airlocks and structure doorways: nothing cuts a hole in solid (PW.3.21b), and a structure is a solid block until its interior exists (PW.4). Open catwalks, bridges, stairs and the open paths are walkable.
- **Measured (native Windows, RTX 4070, release):** 95 to 143 fps in a walk from the default start; 11 200 chunks resident, 0.75 to 1.2 GiB of GPU memory; about 1 200 chunks over the 16 MiB frame budget (uploaded alone); the far band overflows ("far band is full", thousands of instances not drawn). Per L1 cell the generator makes 5 000 to 18 000 primitives (mostly the surface plates), per L2 cell a median of 5 000 meshed boxes (p90 12 700, worst 21 000; the stairs are a third, 171 steps for every slot of climb). Generation lags the camera: after 900 frames the queue still held 8 000 chunks, empty after 1 500.


### 5W.27 Hole cutting (PW.3.21b, `worldgen::carve`)

The clear boxes of bored passages and thresholds are cut out of solid boxes (`carve::carve`): a box is replaced by the boxes that remain outside the holes (disjoint, about five to a hole). Where the piece and the hole are boxes in one frame the cut is exact; where the frames differ the hole is covered by slices (as few as keep each slice's bounds within about twice the hole's own volume, at most six) and each slice by its bounds in the piece's axes, rounded outwards, so the hole can grow a little but never shrink and nothing solid stays in a passage. Pieces that are not boxes (ramps, cylinders) are left whole and counted (`CarveStats::unsupported`).
- **L2 cell:** `l2_cell_web` cuts the structures' boxes by the cell's path and threshold bores (a doorway is a 4 m alcove in the structure's solid until PW.4 builds the interior); the pieces that line the passage are not cut.
- **L1 cell:** a bored passage crosses the L1 solid, and an L1 chunk cannot know the bores of its 512 L2 cells without growing them. The bored segments follow from the routes alone (a bored segment is never swallowed, its type, width and legs do not read the structures), so `l2_web::l1_primitives_carved` runs `tissue_routes` for the 512 cells (about 1.7 s a cell), takes `paths::bored_bores` (the same boxes as the grown cell's, tested) and cuts the L1 pieces with them. The engine builds L1 full chunks from it. A threshold's own clear box is not used for the L1 cut (a structure can grow over the solid slot that gives the face, so the threshold may not exist in the grown cell); the passage's own bore covers the same corridor.
- **Thresholds draw by face** (`THRESHOLDS` + the slot and direction), so a face's type does not depend on what else the cell holds.
- **Measured** (six L1 cells): 80 to 224 pieces cut per cell, 0 to 10 left whole (cylinders and ramps); the pieces of a cell grow by 10 to 140 % (9 600 to 28 000 become 14 700 to 31 500). `a_bored_passage_can_be_walked` follows every horizontal bored passage of an L1 cell along its centre line: a floor under 96 % of the samples (the rest are the 0.2 m gaps of NEAR-04), and 96 % free to 1.1 m above it; **no sample is blocked by an L1 piece**; what blocks the rest are L2 pieces where a leg meets a stair at a corner slot (PW.3.12b).
- Not cut: ramps, cylinders and other non-box pieces; a doorway into a form built of cylinders stays shut.

### 5W.28 Mesh and generation budgets (PW.3.21c, `engine::chunkmesh`, `engine::primmesh`)

- **Measured before** (native Windows): an L1 chunk 19 to 30 MiB (12 000 to 28 000 pieces), an L2 chunk up to 26 MiB; 360 chunks dropped for being over the 16 MiB frame budget; the far band overflowed 9 700 times.
- **Changes.** (1) The mesher merges coplanar quads of one material that share a whole edge. (2) A face that the boxes beside it leave in three or more fragments is drawn whole (the fragments cost more quads than the face; what hides behind a neighbour is invisible). (3) Cylinders have 6 to 24 sides (was 8 to 48): a cell has thousands. (4) The far band draws only pieces at least 32 m thick in every direction (`l1::FAR_MIN_THICKNESS_Q`): about a tenth of the far-visible pieces, which is what a 50 km band can hold; and holds 3 145 728 instances (192 MiB, was 1 048 576).
- **Measured after.** L1 chunk 11 to 12 MiB, L2 chunk median 3.7, p90 7.7, max 11.4 MiB (120 cells): none over the budget; no chunk dropped. Far band: no overflow. A 20 m/s flight over 70 s keeps all 11 200 wanted chunks resident (queue 0, 1 700 evicted) at 126 to 136 fps; a standing start fills the whole 50 km band in about 35 s, nearest chunks first (8 300 of 11 300 at 15 s). GPU memory 0.8 GiB.
- **Not done:** stairs are still one box per step (171 for every slot of climb); the L1 surface plates are still 5 000 to 18 000 pieces a cell (the mesher is where they were reduced, not the generator); the oversize-upload rule stays (a chunk up to four budgets uploads alone).

### 5W.29 Z-fighting and windows (PW.3.21d and PW.3.21e, version 13)

- **Z-fighting.** Turned pieces (every Web frame has a yaw or a tilt) are not culled against each other, so two pieces with a face in one plane fought for the pixel; flat boxes already lose their shared faces in `chunkmesh`. (1) `engine::primmesh::nudge` grows every turned box by 0.25 to 2 mm on each side, the amount from a hash of its own origin and size, so the same piece is nudged the same way in every chunk; skin and windows grow 4 mm more, so cladding stands in front of the wall it clads. Reversed-Z float depth resolves far less than that at any distance the game draws. (2) `l2_web` drops identical pieces (same shape, material, kind and flags: the shared edge of two frames), which no offset can separate. (3) Test `zfight::coplanar_faces`: no identical pieces in sample cells and their parents; it also prints the count of coplanar same-facing visible faces by kind pair, which is large (thousands in 10 cells: frame beams crossing, decks with their rails) and is what the nudge is for.
- **Windows.** A structure window is a 1.4 m by 1.6 m panel on a 4 m grid (one storey), 12 cm thick, about 30 % of grid places lit (hash of the slab and the grid place), instead of four 8 m squares of pure white per slab. The material is the new `LIT_WINDOW` (id 8: warm, emissive 0.4; lamps keep `EMISSIVE`), also used for path glass, the lit panels of path modifiers and the cliff windows. Windows go only on the faces of the structure's box parts: a cylinder, dome, helix or other shaped form fills its bounding box's corners with air, and a window there floated; and not on a face against another solid slot (built, parent solid, engulfed), which has no outside. At most 8 slot faces per box part.
- **Test.** `zfight::windows_lie_on_walls`: of the 3 400 structure window panels in 60 cells, 1 % (36) do not have a solid piece behind and none in front (the bar is 2 %); these are mostly SV-18 (windows only: openings in the parent's wall), whose host is the parent's solid in the parent's frame, not the lattice's. Open: make SV-18 test its host.

### 5W.30 Collisions on the paths (PW.3.21f, version 14)

What keeps a walker from being stopped on a path:

- **Bored steps.** `tissue::SolidProbe` tests a route step against the parent's solid along its whole length, not only at slot centres: a step that runs into a plate or wall no centre lies in counts as bored, so its slots are `crossed` and the L1 solid is cut there. Segments are also cut at junction slots (three or more routes), so stairs and decks meet at nodes.
- **Trim.** `Leg::trim`: rails, walls and posts stop 1.7 m past a corner node (half a width plus 1.7 m), because they stood across the other leg's walkway (`Build::clamp_side`, `side_solid`); applied to rails, walls, posts, PH-11/12 linings, PH-14 walls and the enclosure walls of `modifiers.rs`.
- **Corridors.** `PathPieces::corridors`: each leg's walkway 1 m each side and 4 m high, each stair's footprint, cut out of the structures and the skin's cladding by `l2_web` (turned and tilted parts reach past their slots).
- **Stairs.** A stair's plan is turned so that no leg meets its foot on its solid side (§5W.17).
- **The cut does not read growth.** `paths::bored_bores` runs on the routes alone, so the L1 solid can be cut before a cell has grown: a bored passage's plan reads the segments, not `Tissue::route` (which growth extends), and a bored passage has no wall side. Test `the_cuts_follow_from_the_routes_alone`.
- **Measured** (`l2_web::tests`, one L1 cell, samples along centre lines 0.3, 0.9 and 1.1 m above the floor): open level segments 52 000 samples, blocked 0.13 % (stairs and ramps 16, structures 24, none by L1 solid or rail); bored passages 8 900 samples, blocked 0.4 % (structures 25: a cut that is not a box, or a frame that differs, stays whole). Bars: 0.2 % open (0 by rails), 1 % bored.

### 5W.31 Edge dressing (PW.3.22, `worldgen::edges`, version 15)

`l2_cell_web` ends with a railing pass over the finished cell (`edges::guard_edges`). It reads the cell's own pieces, the parent's solid translated into the cell (cut by the clear boxes of the bored passages and thresholds, as the engine will see it) and returns rail boxes. Rules of §5.4 apply: a drop is more than 15 m, a run of open edge is guarded with `chance(0.85)` (the draw is keyed by the cell, the piece's index, the edge and the run's start, salt `EDGE_GUARD`) or left unguarded on purpose.

- **Decks and landings:** pieces of kind WALKWAY, DECK or BRIDGE that are boxes (0.125 to 1 m thick, at least 0.5 m each way) with standing room over the middle of the top (0.1, 0.9 and 1.7 m clear; a rib under a deck or a plate under a wall has none). Each of the four top edges is sampled every metre at 0.45 m past the edge: it is *closed* if a solid stands 0.9 m above, or a floor within 0.75 m below or 0.3 m above (a floor that goes on, the foot of a stair), *over a drop* if nothing solid stands in the vertical line below down to 15 m, and *already guarded* if a rail or wall stands 6 cm inside the edge at 0.6 or 1.05 m. A long edge whose two ends and middle are closed or guarded is taken as closed.
- **Flights:** each side of a stair flight in pieces of 12 steps, tested the same way at the middle of the piece. A rail piece is a stepped box from the foot of the piece to 1.125 m above its highest tread (a sloped rail is not possible in a tilted lattice frame).
- **The rail** is a box of kind RAILING, METAL, 1.125 m high and 0.125 m thick, flush with the edge, in the piece's frame. Rails of neighbouring deck pieces that lie on one line in a frame and touch (a gap of up to 0.1 m) are joined into one box, so a catwalk built from many pieces gets few rails; the joined boxes are placed in the frame about the cell's origin, so they sit within 3 cm of the edge.
- **Measured** (120 cells): about 1 450 runs above a drop a cell (a third on stairs), 85 % guarded, 1 160 rail boxes a cell added on average, which is 8 % of the cell's pieces plus; the L2 chunk is 4.5 MiB median, 8.9 p90, 12.1 max (was 3.7, 7.7, 11.4: the windows and the rails). The pass costs a median of 39 ms and a mean of 52 ms (worst 0.44 s) a cell, 10 ms of it building the indexes: about two thirds of what the cell cost before.
- **Not done:** guard rails on the roofs and balconies of masses (they are not path pieces), ladders for drops that need them (PW.5), a railing where a drop has a stair beside it. The sampler's centre line of PH-04, 05, 19 and 20 leaves their deck, so the walk test does not count rails against them.
- **Test.** `edges::open_edges_above_a_drop_are_guarded` (the share guarded is in 75 to 95 % over more than 100 runs; rails added at most 2 000 a cell); `l2_web::an_open_segment_can_be_walked` (no more than 1 in 2 500 samples is blocked by a rail).


- **Drawn as it collides (PW.4.9, `engine::primmesh::add_rail`).** A RAILING piece is meshed from its own shape in its own frame, never as a kit module at its world bounds (the v6 `l4::railing_modules` is no longer used by the Web). A box rail gets a post every 2.5 m (10 cm square, plumb in the frame), a top bar and a middle bar along it, and a toe plate; a flight's wedge rail gets plumb posts whose tops are on the wedge's sloped top, with the two bars raked parallel to it. Collision keeps the whole box or wedge, so the look is open and the collision is a parapet (the walker cannot slip between posts). The foot box of a flight rail is a level rail across the whole flight (PW.4.12 replaces it).

- **Rails follow gravity (PW.4.10, `worldgen::plumb`, version 18).** Up is world +Y, not the frame of the piece. Only a surface whose normal is within 40 degrees of +Y (`plumb::walkable`) gets rails, and the drop test, the standing-room test and the probes of the edge pass use +Y. A rail is built in the *upright frame of the piece's heading* (`Table { yaw, 0 }`, or the horizontal `Dir`; `plumb::upright`): the same footprint, a height of 1.125 m measured from the walking surface, never leaning. A bar along the climbing axis of a tilted frame is cut into steps of at most 0.5 m that follow the slope (the height of a step spans the surface under it). Posts and wall-like rails stand as one plumb box per step. `Emitter::frame_box` does this for every RAILING box a path emits in a tilted lattice, and `edges.rs` for decks and flights. A flight in a tilted frame climbs along the frame's slanted up, so its line is not in a plane of the upright frame: its rail is a row of plumb steps of 1 m along the line of the nosings (`stepped_rail`); a flight in an upright frame keeps the foot box and the wedge (one wedge, raked with the flight, plumb posts in the drawing). Headroom of a walk (`l2_web::walk_l1`) is measured plumb too. A lattice whose floors are not walkable gets no rails at all.
- **Rails never cross a way on (PW.4.11, `edges::cut_rails`, `l2_web::ways_of`).** (1) An edge of a deck whose outward side is outside the cell (more than the 0.45 m probe past the cell's box) is closed: the neighbour decides, so a deck that ends on the face has no rail across its end. (2) `cut_rails` takes every RAILING box in an upright frame and cuts it where a way on crosses it: the ways are the walking lines of every leg and rise (`PathPieces::walks`, half the walkway width) and every doorway (4 m wide, 2 m each side of the wall, `l3_web::doors_of`). A rail stops 0.3 m short of a way (sampled every 0.25 m, a way counts within 2 m of the rail's base height); a rail parallel to the way (within 14 degrees) is the way's own side rail and is only kept off the inside of the half width; stretches left under 0.5 m go; a post or stub is kept whole if its middle is clear. Rails of the neighbour's ways are not read: where legs meet at a face with different widths a rail can still stand across a neighbour's wider walkway (2 cells in 300 in the census; item PW.4.28).
- **Stair sides (PW.4.12, `paths_v`, `edges`).** `paths_v::track` is a row of boxes 15 cm high and 30 cm wide, one on each tread (sunk 2 cm into it), instead of a wedge laid on the slope (its slope crossed every step in a sawtooth). `handrail` is pieces of two units, each a level slab 1 m over the higher tread with a post standing on its tread (was ten units in one level slab, floating up to 1.7 m above the low end). `ramp` adds its footing only under the first metre of the run (was a plate under all of it). The rail of a flight stands one quantum inside the side of the flight, so no face of it lies in the plane of the flight's side.

- **A helix has rails and a stair's column is not a house (PW.4.13, `paths_v::helix`, `l3_web::mass_specs`).** A helix (PV-05) gets a rail on its inner and on its outer edge, a plumb box 1.125 m high to each tread in the tread's own heading; the two treads next to each deck are left open, and `cut_rails` opens the way in. A part that the clear box of a rise meets is not a mass: it stays solid (`PathPieces::rise_clear`, lattice-frame metres) and is cut for the stair, instead of an interior with a stair through its rooms. Open: the column is not reserved before hubs, sets and structures are placed, and `carve` cannot cut cylinders, ramps and stairs.
- **Dressing goes with its host (PW.4.14, `edges::orphan_mask`).** The last pass of `l2_cell_web_edges`: a `NO_COLLIDE` window, lamp, glyph or cable with no host in the final geometry is dropped. A host is a collidable piece of the cell or the parent's solid that no bore cuts, within 10 cm of the piece's bounds (nine points); a cable needs it at both ends unless the end is on the cell's side. Four fifths of the cell's dressing had none (cables ending in air, lamps in the air): it is gone, not moved.
- **Supports reach something (PW.4.16, `edges::fix_members`).** A plumb post, frame, pier or hanger (box at least 1 m high, at most 0.8 m across, three times as tall as wide) that touches nothing at either end is extended down or up, whichever meets solid first within 8 m, by that distance plus 5 cm; one that meets nothing within reach is dropped. Members within 1 m of the cell's side are left alone.
- **Every door has a way to it (PW.4.17, `l3_web::floored_slots`, `approach_decks`, `door_cuts`).** A route entrance (door, not hatch) exists only if there is a floor under the route where it passes the entrance slot's centre (`Tissue::floored_slots`: 2 m round the centre, from the paths and payoffs); the mid-flight levels of a stair that passes a structure have none and get no door (no threshold either). Each kept door gets an approach: a deck at the sill, as wide as the doorway (4 m), from the middle of the route slot to the face, kind WALKWAY, part of `Tissue::paths.prims`. The door's cut box (3 m high, 4 m wide, 2 m through the wall) is also a bore for the cladding and the other structures (`l3_web::door_cuts`), not only for the shell. v6 `meshpool::doorway_of` and `hollow_envelope` act only on `Entrance` ports, which the Web L2 never emits (test `web_l2_cells_have_no_entrance_ports`): kept for the ignored v6 tests. `mid_primitives` keeping only envelopes is the mid band's silhouette by design, not a leftover.

### 5W.32 The defect census (PW.4.8, `worldgen::defects`, `web-report --defects`)

A test-side measurement that changes no output. `scan_cells` takes L2 cells; for each it builds one cell-local frame holding the cell's own pieces, the pieces of all its L3 world cells (moved into the cell), the parent L1 pieces that reach it, and the path, threshold and payoff pieces of its 26 neighbours (as witnesses only). A piece belongs to the cell that holds the centre of its bounds. `PathPieces::walks` records the walking line of every leg (two ends) and rise (samples), with the frame's up. Counters:

| | What it counts | Rule |
|---|---|---|
| a | rails with no floor beside them, and `a lean`: rails whose frame is more than half a degree off plumb | a RAILING box at least 1 m long (horizontal) and 0.6 m high whose base has no collidable non-rail solid 0.6 m to either side at the base, at two of three sample points along it |
| b | solid path pieces in a walk space (`b rail` for rails, `b kind N` for the rest; the pieces built for that walk itself are not counted, and neither are samples within half a width plus 1 m of a leg's ends or the first and last 2 m of a rise, where it meets what it leads on to) | pieces of paths, thresholds and payoffs (own and neighbours') that contain a point 0.3, 0.9, 1.5 or 1.8 m above a walk line (a leg: 3 points across its width, every metre; a rise: its samples) |
| c | dressing with no host | a `NO_COLLIDE` L2 piece other than MISC and VOID that has no collidable piece of another kind touching its bounds grown by 10 cm (8 points); a cable needs a host 10 cm beyond each end, unless the end is on the cell's side |
| d | vertical members with nothing at either end | a collidable box at least 1 m high, at most 0.8 m across and 3 times as tall as wide, with no other solid 5 cm below or above its centre; members within 1 m of the cell's side are not counted (their end is in the neighbour) |
| e | doors with no floor (doors within 1 m of the cell's side are not counted) | a Door entrance: no collidable piece at floor level (or 0.2 m below) 2 m outside the face (`e outside`) or 2 m inside (`e inside`) |
| f | path ends with nothing to continue on | a leg end with no floor 0.8 m beyond it or half a width plus 0.8 m to either side: `f solid` when something solid stands 0.6 m beyond at 1 m, else `f air`. `f rise`: the first and last sample of a rise with no collidable piece that is not the rise's own 1.5, 2.25 or 3 m away on any of 8 headings, 10 cm under the end. Dead ends that carry a payoff (PW.3.18) are counted too |
| g | coplanar visible faces | faces of a drawn piece (not RAILING, VOID, a lift shaft or a non-interior envelope) whose sample point has the piece inside and nothing outside, but another drawn piece behind it in the same plane; plain boxes and stair steps of one chunk are skipped, since `chunkmesh` culls them. Reported by level pair, and `g stair` for pairs of L2 pieces where either is a stair or a ramp |
| h | tilted interior floors | an L3 slab (at most 1 m thick, 1 m wide each way, normal within 40 degrees of up) whose normal is more than 1 degree off up |
| r | pieces in a rise's clear volume | a collidable piece of L2 or L3 (not a rail, not a path piece: other paths meeting the stair are a junction, not an overlap) that contains a point of the clear box of a rise (grid of 1.5 m); L1 is not read here (the engine cuts it with the bores) |
| i | rails not drawn where they collide | a RAILING that is neither a box nor a wedge (shape or frame), which `primmesh::add_rail` cannot draw in its own frame. Before PW.4.9 the engine drew every rail as a level kit module at the world bounds (`l4::railing_modules`), so turned, tilted and raking rails were drawn elsewhere (111 697 of 1 599 096 rails in the baseline) |

Each count comes with the number measured (`[a rails]`, `[b samples]`, ...) so a rate can be read. Rules for the counters are deliberately generous (a floor anywhere within the probe counts), so a count of 0 is a real 0. `web-report --defects N --seed S --origin x,y,z --size m` samples N cells from the region; the test `defects::tests::census_runs` runs one cell (set `DEFECT_N`).

### 5W.33 Far signatures and the far-vs-near measurement (PW.7.1, PW.7.2, `GEN_VERSION` 21)

**Knot signature (HOR-07).** `Emitter::knot` draws one of eight silhouettes from `(signature >> 2) & 7`: 0 monolith, 1 spire on a base, 2 twin pylons with a bridge, 3 square ring of beams, 4 ziggurat, 5 cross of long bars on a core, 6 disc on a stem, 7 crown (core and four spikes); the size is `48 + 16 · (signature & 3)` m; the lamp on top is one of four lights from `(signature >> 5) & 3`: amber (`BEACON`), cold white (`TRAIN_LIGHT`), white (`EMISSIVE`) and a red signal (`SIGNAL`, new material id 9). Every piece is a `FAR_VISIBLE` box at least 32 m thick, so the far band draws all of them, and stays near the knot's own L2 cell. The knot's hub behind the signature (HOR-08) is `Skeleton::knot_hub`, the route (HOR-09) is the artery tree rooted at the knot, and there is one knot per L1 cell, so landmarks are 4 km apart (HOR-10). A well's furnace forces the knot of its L1 cell (`gravity.md` §3.3). The blink rhythm is data in the signature and is not drawn yet.

**Far-vs-near measurement (HOR-03, HOR-06).** `crates/gen/tests/far_near.rs` (ignored; release): first-hit depth maps of the boxes along the three axes at 16 m pixels for one L1 cell: far (the L1 primitives, as drawn at 5 km), mid (plus the L2 envelopes, 3 km) and near (everything, 300 m); two surfaces within 48 m are the same. Result over four L1 cells on 2026-10-21: the near view shares its first surface with the far view on 4–48 % of the pixels, the far view has nothing where the near view has a structure on 9–54 %, and the surfaces differ on 20–49 %. The far view never has something the near view lacks. **This fails HOR-03:** L2 structures are large (hundreds of metres) and L1 draws no mass for them, so they appear at the L2 mid radius. The remedy is an L1 mass proxy from the fill field (PW.7.1b); nothing is hidden by the measurement, and the test only prints.

**Far mass (PW.7.1b, `GEN_VERSION` 28).** `l1_fill::far_mass` adds to every L1 cell one far-only, far-visible, passable box per run along x of L2 cells whose fill is at least 24 of 255 (cells of fill `f` hold a block of edge `512 (f/255)^(1/3)`, found by integer search in 8 m steps, at least 32 m; the box is as long as the run, centred in its cells). It is the parent's answer to what its children will make, derived from the fill field only, so L2 still refines it and never contradicts it. The full L1 mesh drops it (`FAR_ONLY`); the Far chunks beyond `l1_full_radius_m` (4096 m) draw it, and the L2 mid radius is now 4096 m so that no band lacks mass. **The measurement compares silhouettes**, as the proposal says (§11.2): the occupancy of the three axis views downsampled to 64 m cells, both over either. Over the four L1 cells of 2026-10-21: 99.2, 90.6, 92.0 and 95.3 %; the test asserts at least 85 %. First-surface depths still differ by more than 48 m on a third of the pixels, which is inherent: a structure stands anywhere in its 512 m cell and the proxy does not know where. **Far lights (HOR-05, version 31).** Each run of mass carries, in 85 runs of 100, two strips of lit windows 2 m tall along its faces towards -z and +z (`l1_fill::far_mass`, material `LIT_WINDOW`, far-only). The near view lights 76 to 98 % of its built cells (the kit, the quarters and the paths all put lit windows on façades; only 1.3 % of structures are themselves `lit`), and the far view lights 75 to 88 % of its runs; the test asserts at most 25 points of difference. They match in rate, not in place. **The SDF horizon beyond 51 km is off by default** (`Settings::horizon`): it marches the rules of the old trunk-and-deck world, and a far structure that is not the one you reach is the worst bug (north star), so beyond the far band there is only fog until it is ported.

### 5W.34 The exterior kit (PW.8.1, PW.8.2, `worldgen::kit_ext`, `GEN_VERSION` 21)

`kit_ext::dress(&tissue)` adds visual relief to the box parts of structures: for each part (all `Structure::parts`, in the lattice's own frame, so turned lattices carry it turned) three draws pick a kit piece and a face; a part takes the kit with chance 1/2 and only if it is at least 8 m tall. Every piece is `NO_COLLIDE`, at most 3 m deep, and is dropped if it comes within 0.5 m of a bore (`t.paths.bores`, `t.thresholds.bores`). Salt `KIT_EXT` (`0x0318`), keyed by the L2 cell and mixed with the structure and part indices. Round things (masts, pipes, cables, fans) are square bars: the emitter makes boxes in the lattice's frame.

| ID | Piece |
|---|---|
| KIT-01 | a strip of lit windows (1.2 m tall) at a storey or a field of 1.4 × 2 m panes on a 3 × 4 m grid (`LIT_WINDOW`) |
| KIT-02 | a cornice at the top (1.2 m out, 0.5 m) or a ledge at a storey |
| KIT-03 | vertical ribs every 8 m (0.6 m) |
| KIT-04 | a vent grille (3 × 2 m) with a fan |
| KIT-05 | a mast of 8–24 m on the roof with a lamp on its tip |
| KIT-06 | one to three façade pipes the height of the face |
| KIT-07 | cable runs across the face at two heights (`CABLE`) |
| KIT-08 | a coffered underside: beams every 4 m in both directions under the part |
| KIT-09 | one to four hanging lamps on chains from the underside |
| KIT-10 | a glyph panel: a 3 × 4 m slab with three to five lit bars |
| KIT-11 | a handrail along the foot of a face on posts every 2 m (stairs keep their own rails, PW.4) |
| KIT-12 | a scaffold: poles every 3 m and a grating deck every 8 m, up to 24 m |
| KIT-13 | every ladder family: built in PW.5 (`ladders.md`) |
| KIT-14 | gravity-gate rings: built in PW.6.3 (`gates.rs`) |
| KIT-15 | furnace machinery: in the L2 cell that holds a well's centre, eight boilers (16 m, 36 m tall, solid) on a ring of radius 200 m with a square pipe run at 10 m (`wellgeom.rs`) |

### 5W.35 The infinite staircase in the Web (PW.8.3, `worldgen::l1_stair`, `GEN_VERSION` 21)

The rare staircase (`regimes.md` §5.2: one L1 column in about one L0 column in a hundred, its footprint a 64 m square at a 16 m grid position, the same in every L1 cell of the column) is built again for the Web. Each L1 cell of the column holds 128 flights of 128 steps (0.25 m rise, 0.375 m tread, 4 m wide, 32 m of rise over 48 m of run) and 128 corner landings of 8 × 8 m, so a turn takes 128 m and a cell holds 32 turns.

- **STAIR-01 open flights.** Each step is a tread plate of 0.125 m and each group of four treads has a stringer of 0.3 m under each edge (a stepped beam of 1.5 m), so there is air beneath. No `StairRun` wedge is used. A cell holds 24 576 flight pieces; the tread depth of 0.375 m is narrower than the walkability validator's plates, so the stairs are walked by the controller and not certified by `walk.rs` (they are not on any connectivity argument).
- **STAIR-02 core.** A 32 m square core in the middle of the footprint, chosen by the column (`STAIR_STYLE`, `0x0319`): **glazed** (a solid column with a row of lit panes 2 m tall every 32 m on its four faces), **lattice** (four posts and a ring of beams every 32 m) or **trunk** (four walls of 4 m round a hollow). Nothing else stands in the footprint: the L1 pieces that enter it are cut (boxes) or dropped (cables and turned beams that really enter it, `l1_web::enters`), and the cell lists the footprint column as a `KeepOut`, which the tissue reads as void.
- **STAIR-03 round spiral.** One column in three has a real round spiral instead: 64 faceted treads a turn on a radius of 24 m, 4 m wide, 0.4375 m rise (28 m a turn, 146 turns a cell), `orient::helix_stair`. The core is the same.
- **STAIR-04 far band.** A core column of 32 × 4096 × 32 m and four corner pylons of 32 m are `FAR_VISIBLE | FAR_ONLY | NO_COLLIDE`: the far band draws them, the L1 full mesh leaves them out (new `PrimFlags::FAR_ONLY`, `1 << 5`), and the L2 parents ignore them. **Deviation:** the stringer silhouette is the four pylons, because a stringer is thinner than the 32 m the far band draws.
- **Reach.** The skeleton ties the footprint's L2 cells to the tree (`TieKind::Stair`), so artery routes lead to the column; the flights themselves have no doors.

### 5W.37 Quarters (PW.10.4, PW.10.5, `worldgen::quarters`, `GEN_VERSION` 23)

A stratum's character has two to four dominant quarters (SC-02). Each L1 cell picks one of them (`QUARTER`, `0x031A`, keyed by the cell), so a quarter spans 4 km. `quarters::dress` adds that quarter's dressing to the box parts of the structures of its L2 cells (half of the parts, two faces each, at least 8 m tall; pieces within 0.5 m of a bore are dropped), visual only, in the lattice's frame like `kit_ext`: QTR-01 dwelling warren (balconies and small lit windows in a grid), 02 foundry (chimneys with a glowing rim, a glowing channel along the foot, pipes), 03 data (racks of small lights, antennas), 04 transit (platform edge, signal boards, catenary posts), 05 archive (shelving), 06 hydroponic (planter strips with grow-lights), 07 necropolis (niches with candles), 08 administrative (a colonnade with an entablature), 09 bastion (merlons and buttresses), 10 dock (a gantry crane on the roof), 11 waterworks (tanks and manifold pipes), 12 observatory (a stepped dome with a lit slit and a telescope), 13 market (awnings, stalls and lamps), 14 chapel (tall lit windows and a spire), 15 hospital (a lit cross and a ramp), 16 machine (a wheel on the face and a bank of pistons). **Not built:** a quarter does not change the weights of forms and set pieces (the character's own weights do that), and the fills (QTR-specific structure families) are the next pass.

### 5W.38 The City's script and names (PW.10.6, PW.10.7, `worldgen::glyphs`)

**MUL-01.** The world has an alphabet of 32 glyphs of 7 × 7 cells, each drawn from three to six strokes by `GLYPH` (`0x031B`, keyed by the glyph's index and a retry counter), all distinct, 8–30 lit cells each; a function of the seed. A **name** is three glyphs (15 bits). The glyph panels of the exterior kit (KIT-10) show the name of their L1 cell, lit. `slice --glyphs --seed N --out out/glyphs.png` draws the alphabet and eight names (TOOL-05).

**MUL-02.** The name of an L1 cell is `k · (x + 17 y + 289 z) + s` modulo 2¹⁵, with `k` odd and `s` drawn from the world (`NAME_PERM`, `0x031C`): a bijection of the lattice sum, and for differences of at most 8 on every axis that sum is not 0 modulo 2¹⁵ (17 > 8 and 289 > 8 · 17 + 8), so **no two L1 cells within 32 km of each other, and so no two hubs, have the same name**. Names do repeat farther away (there are 32 768 of them). Test `no_two_cells_within_thirty_two_kilometres_share_a_name` over a region of 25 × 25 × 25 cells (100 km).

### 5W.39 Overgrowth (PW.10.24, `worldgen::growth`, `GEN_VERSION` 24)

Visual relief on the box parts of structures, `NO_COLLIDE`, in the lattice's frame (like `kit_ext`), keyed by the L2 cell and mixed with the structure and part indices (`GROWTH`, `0x031D`); a part draws all its chances whether or not it takes them. Two new materials: **MOSS** (id 10, a dull olive grey) and **FUNGUS** (id 11, pale and glowing a little). A part of the Hydroponic (QTR-06) or Waterworks (QTR-11) quarter takes about twice as much. **GROW-01** moss on the north (−z) face, in clumps of 2 m slabs, at most 24; **GROW-02** roots hanging from the underside, 3–12 m, six a part; **GROW-03** pale fungus caps along the foot of a face, five a part; **GROW-04** vines of 3–6 m hung from the top edge of a face every 2.5 m, in the damp quarters only. **GROW-05** a tree of pipes that is a tree: one L1 cell in forty holds a trunk of 12 m and 160 m, eight roots, and three tiers of six boughs of pipe rising 15°–30° and thinning, each ending in a crown of moss; its volume is kept clear (a feature in `features.rs`, `growth::tree_content`).

### 5W.40 Forms, batch 2 (PW.10.25–PW.10.27, `worldgen::forms2`, `GEN_VERSION` 24)

F2-01..24 are appended to `forms::FORMS` with their own tags and the families of their nearest cousin (they are drawn like FORM-01..23, and a form that cannot be made at its size falls back to the slab tower). Sizes in the registry are in slots of 32 m: F2-01 **capsule tower** (a core with pods plugged on alternating faces), 02 **habitat stack** (boxes turned a quarter from tier to tier), 03 **arcology pyramid** (many terraces over a deep core), 04 **cooling tower** (rings narrowing to a waist), 05 **dish** (a shallow inverted cone on a mast), 06 **gasometer** (a drum in a frame of four posts), 07 **blast furnace** (a cone on a drum with four pipes), 08 **viaduct** (a deck on piers every 3 slots), 09 **plug-in megaframe** (columns, floors and plugged boxes), 10 **space-frame canopy** (a cage with a wide roof), 11 **metabolist trunk** (a core with pods on both faces at most levels), 12 **stepped slab** (setbacks on one side), 13 **Boullée hall** (a great dome on a ring of columns), 14 **Ledoux sphere house** (rings that swell and narrow), 15 **cathedral** (nave, aisles and two towers), 16 **monster building** (a dozen heaped boxes and three turned ones), 17 **battleship island** (a long hull with towers in a row), 18 **bridge house** (two legs and a building across them), 19 **silo bank** (cylinders on a roof deck), 20 **lift tower cluster** (four slender towers with skybridges), 21 **stair tower field** (towers with stairs on their sides), 22 **hollow mountain** (heaped terraces, dark, with a bored passage), 23 **hanging garden tower** (terraces and vines), 24 **lattice ghost** (a cage with half its members gone). Test `every_form_builds_inside_its_size`; the tissue's variant test counts every form.

### 5W.41 Paths, batch 2 (PW.10.28, PW.10.29, `GEN_VERSION` 24)

**Horizontal (P2-01..12, ids 61–72, `paths.rs`).** Built with the same `Build` helpers as PH-01..20 and chosen by the same contexts and family weights: **P2-01 pipe interior** (61, 2–4 m; floor, walls and roof of pipe, flanges every 6 m; open air), **P2-02 gutter** (62; a channel with walls of 1.1 m, a lip and a lit drain strip; open air), **P2-03 track walk** (63; planks between two rails with sleepers every 2 m; open air), **P2-04 tram-car bridge** (64, 2.4–3.2 m; a floor, glazed sides, a roof, a bulkhead every 24 m; over a gap), **P2-05 chain walk** (65, 1–1.6 m; planks of 0.9 m at 1 m on two chains that are rails; over a gap), **P2-06 colonnade walk** (66; columns every 4 m on one side under an entablature; beside a wall), **P2-07 cornice walk** (67, 0.9–1.4 m; a ledge from a wall with no rail on the open side), **P2-08 in-wall passage** (68; lined passage with a lit roof strip, bored through solid, in bored and engulfed segments), **P2-09 cable-bundle tunnel** (69; sixteen cables round the walkway and rings of steel, bored), **P2-10 glass-floored bridge** (70; panes of lit glass that collide, in a frame; over a gap), **P2-11 covered market street** (71, 6–10 m; stalls with awnings and lamps on both sides, roof beams every 8 m; between two walls) and **P2-12 ceiling walk** (72; panes between ribs on a roof). `every_type_is_walkable` holds for all of them (a floor under the whole walking line and 1.85 m of headroom). **Deviation:** P2-12 is a walk on top of a roof with glass between ribs, not on a ceiling in an inverted frame.

**Vertical (P2-13..25, ids 73–85, `paths_v/p2.rs`).** Each climbs as a base type of the first batch and is dressed with pieces that are not solid: **P2-13 rolling ladder** (73: LAD-01 with shelving and a rail along the top), **14 dumbwaiter shaft** (74: LAD-01 with shaft walls and a parked box on its rope), **15 chimney climb** (75: staples in a ring of brick slabs), **16 stair-escalator** (76: stairs with machinery housings and a lit strip), **17 counterweight pair** (77: stairs with two cars on cables), **18 Ferris cabins** (78: stairs inside a wheel of eight cabins), **19 bucket chain** (79: a ladder chain with buckets on a loop), **20 funicular** (80: a stair beam with rails and a car), **21 spiral of spirals** (81: the helix with a ring of rising lights), **22 grand ramp** (82: the ring ramp with lit parapets; the column is 32 m, so not 200 m), **23 stepped pyramid stair** (83: terraces round the stair), **24 inverted stair** (84: the stair hung from four rods to a slab), **25 tether climb** (85: a ladder up a thick cable with a lamp). **Deviation:** nothing here moves (the cabins, buckets, cars and counterweights are parked, and the movers are PW.10.12); the walking part is always the base type's.

### 5W.42 Payoffs, batch 2 (PW.10.30, PW.10.31, `worldgen::payoffs2`, `GEN_VERSION` 25)

At a dead end the payoff pass (`payoffs.rs`) now tries, after a set piece and before a view, an **installation**: one leaf in four takes one of ten (`PayoffKind::Install(n)`, the kind drawn from the leaf's second word, its own draws from a generator seeded by two of the leaf's words). Each stands in the slot's 32 m cube, within 16 m of its middle, and most are not solid. **PAY-01 anamorphic glyph:** the first glyph of the district's name (`glyphs.rs`) as boxes of light on the rays from the point where the walkway ends, at 6–15 m, sized with their depth, so that from the focal point they cover the 7 × 7 cells of the glyph (a plane 12 m away, 1 m a cell, raised so the lowest row is above the eye) and from any other point they are scattered; test `an_anamorphic_glyph_resolves_from_its_focal_point` projects every box back and finds it on a lit cell. **PAY-02 cartographer's room:** a table of 8 × 8 m with a relief of 64 columns whose heights are the real fill field of this level of the district (`0.05 + 0.5 · fill`; test compares them). **PAY-03 focal window:** a wall of 12 × 6 m on the side towards the district's knot with a slit of 2 × 2.5 m. **PAY-04 the previous walker:** a figure seated in the corner, a lantern beside it. **PAY-05 a lit room:** a closed room of 6 m with a lit window and a warm lamp. **PAY-06 collection:** a room of 400 lamps (20 × 20, one a metre) hung at their own heights from a ceiling at 8 m. **PAY-07 message:** a wall panel with eight glyphs in two rows, a line of the City's script. **PAY-08 echo chamber:** a ring of seven wall slabs (one gap for a door) and a roof (the echo is PW.11). **PAY-09 seat:** a bench and its back. **PAY-10 bell:** a shaft of 5 m and 22 m with a beam, a rope and a bell of four stacked boxes (the ring is PW.11).

### 5W.43 Sanctuaries, batch 2 (PW.10.35, PW.10.36, `worldgen::sanctuaries`, `GEN_VERSION` 25)

SAN-01..13 are set pieces: `sets.rs` appends them to `SETS` as indices 16 to 28 (tags `SAN-nn`), they are placed like SET-01..16 (against a dead end by the payoff pass and early in step 3 by context, in the free slots), and each is lit. Weights: SAN-03, 04, 05, 09, 10 and 13 anywhere (1); SAN-02 and 11 where the fill is at least 128 (1); SAN-01 (2), 06, 07, 08 and 12 beside a void (1). **SAN-01 tea room over a Between:** a slab with a room set back on it and a terrace. **SAN-02 greenhouse under a small lamp sun:** a low hall with a dome. **SAN-03 library nook:** a small room with shelving walls. **SAN-04 lamp chapel:** a nave with a lit spire. **SAN-05 bunk room:** a low room with three tiers of bunks (the murmuring radio is PW.11). **SAN-06 observatory:** a room with a dome (the telescope is PW.10.17). **SAN-07 needle top:** a thin shaft with a room at its top. **SAN-08 gondola cabin parked at a tower:** a tower with a small cabin hung beside it (the gondola itself is a mover, not built). **SAN-09 pool room:** a hall round a sunk pool (the water is not drawn). **SAN-10 bell room:** a room with the bell's drum. **SAN-11 garden in a light well:** a ring wall with vines. **SAN-12 hanging pod:** a capsule under a slab. **SAN-13 reached only by a long ladder or a featherfall:** a room on a pillar of 6 to 10 slots with a cage up its side for the ladder; a shaft of the featherfall kind in the same column is the other way. **Earned sanctuaries:** not built as a rule. A sanctuary at a dead end is a payoff of that route, but its chance does not grow with the length of the route to it; the weights above are all there is. This is recorded in PW.10.36.

### 5W.44 The walker's tools (PW.10.16–PW.10.18, `engine::tools`, `worldgen::landmarks`; no `GEN_VERSION` change)

Engine-side, saved in `save.toml` (`marks`, `places`, `trail`, `chalk`). `landmarks::near` lists the
wonder, rare, mover and spine sites of the L0 cells around a position, a function of the seed.

- **TOOL-01 zoom.** Hold Z: the view narrows 4× (`tan(fov/2)/4`). **TOOL-03:** in the Observatory
  quarter (index 11, `quarters::quarter_at_l1`) the same key gives 16×. *Deviation:* the
  Observatory quarter as a whole is the telescope room; no separate eyepiece fixture.
- **TOOL-02 gaze-mark.** M pins the landmark best aligned with the view (cos ≥ 0.995, within 400 L1
  cells), or releases it if it is already pinned. At most 8; the oldest is dropped. Each pin is a
  line at the lower right: its name, its distance and its bearing (ahead, left 40°, behind).
- **TOOL-04, TOOL-06 atlas.** Staying 4 s in an L1 cell records it as a place (up to 4096). N opens
  the atlas: the 12 latest places as their transliterated names (`glyphs::transliterate`, 32
  syllables), their quarter and their cell.
- **TOOL-07 listening.** Standing still 2.5 s, outside a Silent region, gives the direction of
  the nearest landmark within its hearing (spine 16 cells, wonder 12, machine 10, oddity 8):
  "you hear a slow grinding, right 128°".
- **WAKE-01..02 trail** and **NOT-04 lamps ahead** were removed on 2026-10-22 (PW.13.0): the user found lamps spawning around the walker unsuitable and annoying. The trail is no longer recorded; a `trail` key in an old `save.toml` is ignored.
- **TOOL-08 chalk.** C draws a bright mark on the surface in view within 6 m.

### 5W.45 Machines that notice you (PW.10.15, `GEN_VERSION` 26)

Four things that answer the walker. Nothing about the world's *generation* depends on the walker:
the machines are generated like any mover, and only their pose is driven by the engine.

- **Sensed movers** (`worldgen::movers::SENSED`, drawn with the other mover kinds at the same 1 in
  10 L0 cells, so existing kinds' sites move at 26): their pose is `pose_sensed(awake, spun_ms)`,
  not `pose(t)`; the world clock never moves them.
  - **NOT-01 EngineHall** (`NOT-01`): a floor of 100 m with pillars and a ring of beams, and a rotor
    of four blades 14 m up (passable). The rotor wakes when the walker is within 90 m of the
    hall's middle, takes 1.5 s to reach full speed, turns one 1/64 step per 250 ms (16 s a turn),
    and spins down 6 s after the walker leaves.
  - **NOT-03 Doors** (`NOT-03`): a gallery of 200 m with four doors 40 m apart, each a leaf 12 × 8 m
    that rises 9 m (smoothstep over 1.5 s) when the walker is within 24 m and sinks 6 s after they
    leave. Each door is its own mover with its own key.
  - The engine (`MoverSystem::sense`) keeps `awake`, the time it has run and the linger per sensed
    mover; these are not saved, so a restart finds every machine asleep.
- **NOT-02 lifts that come** (`liftsys`): waiting within 3 m of a served opening for 1.2 s calls the
  lift as the call key would; it then rests 8 s before noticing again.

### 5W.46a L1 face ports reach the right L2 cell (`GEN_VERSION` 30)

`ArteryPort::l2(high)` is the L2 cell next to the face in the L1 cell on its low side (`high`: index 7
of the axis) or on its high side (index 0). The artery routing (`l1_artery::skeleton`) and the
tissue's anchors (`tissue::tissue_cell`) now use `p.l2(dir.is_positive())`: the cell of the L1 cell
that touches the face. Before version 30 they used `!dir.is_positive()`, which put a port on the
cell at the far end of the L1 cell, so no route crossed an L1 face (the bots measured the largest
connected component at 13 % of the nodes; it is now about 80 %).

### 5W.46 Ambient movers, lamp suns, signs, pools (PW.10.19–PW.10.23, `worldgen::ambient`, `GEN_VERSION` 27)

`ambient::site_of`: one L0 cell in six holds an ambient mover (a `Mover` on the world clock, same
engine system as §3W.10), drawn by weight: lighthouse 3, searchlight 2, light river 2, aurora 2,
steam vent 3, flock 2, crawler 3, spider 2, moths 3, lantern 1. All parts are passable. Fixed
supports are static content in the cell (`mover_content` merges them), with a keep-out.

| ID | Kind | Moving part | Cycle |
|---|---|---|---|
| LGT-01 | Lighthouse | a beam of 2.5 km at the top of a 118 m tower, a turn of 64 steps | 32 to 96 s, its own rhythm |
| LGT-02 | Searchlight | a beam of 600 m sweeping 24 of 64 turns and back | 2 to 4 min |
| LGT-03 | Light river | twenty pulses 50 m apart along a cable of 1 km, one spacing per cycle | 1 to 2 s |
| LGT-04 | Aurora | 24 curtains hung 600 m wide, swaying 3 of 64 turns | 3 to 5 min |
| WX-06 | Steam | a plume of five puffs, hidden 10 km below, rising out of the grating for 40 % of the cycle | 20 to 60 s |
| LIFE-01 | Flock | 24 drones on a ring of 60 to 100 m, a turn | 90 to 150 s |
| LIFE-02 | Crawler | a lamp-carrying crawler on an 80 m pipe, there and back | 2 to 4 min |
| LIFE-03 | Spider | a spider on a net of 20 × 17 m | 90 to 150 s |
| LIFE-04 | Moths | eight moths around a lamp, a turn | 16 s |
| LIFE-05 | Lantern | a lantern on a catwalk of 700 m at 1.4 m/s, always one way | 500 s |

*Deviations:* the lighthouse is seen only within the movers' 3.5 km, not 20 km (its tower lamp is
far-visible); a turn is 64 steps, so a beam jumps; the flock wheels in 64 steps; a mover's
pieces that appear and disappear (steam) are moved far away rather than faded.

- **LGT-06, LGT-08 lamp sun** (`lamp_sun_content`, 1 L1 cell in 24, far-visible): a disc of 60 m
  hung 700 m above a crust of 400 m (16 squares, at least two of them holes) and a shaft of pale
  light (`VAPOUR`, passable) under each hole down to the floor. The god rays are geometry.
- **LGT-10 emergency light:** in a Bastion quarter (QTR-09) a red `SIGNAL` lamp every 6 m along
  the foot of a dressed face.
- **LGT-11 neon:** in a Market quarter (QTR-13) a sign of three glyphs of five by seven cells in
  the new material `NEON`; unreadable. *Deviation:* the buzz waits for PW.11.
- **LGT-09 blackout strata:** the existing Blackout regime event (`atmosphere`: no emissive colour
  at all), with the trail lamps (§5W.44) and beacons as the only light.
- **WX-11 still pool** (`pool_content`, 1 L1 cell in 30): a cistern of 64 m with walls of 3 m and a
  pool of 2 m of coolant whose surface is a flat plate of glossy metal. *Deviation:* no planar
  reflection (PW.10.20b).
- New materials 12 `NEON`, 13 `AURORA`, 14 `BEAM`, 15 `VAPOUR`; salts `AMBIENT_SITE` 0x0130,
  `WEATHER` 0x0131, `FOG_SEA` 0x0132, `FLASH` 0x0133, `LAMP_SUN` 0x0134, `POOL` 0x0135.

### 5W.47 Weather and air (PW.10.19, PW.10.20, `worldgen::weather`, `engine::weather`)

`air_at(seed, p, t_ms, between)`: the air at a point, a function of position and the world clock.
Regions are blobs (a centre in an L1 cell and a radius below 4096 m) looked up in the 27 cells
around the point; coverage rises smoothly to 1 at the centre.

- **WX-01 fog sea:** one L0 cell in four has a cloud top at 2 to 28 km above its floor; inside a
  Between below the top the fog is thicker by 12× over the last 80 m, spires stand out above.
- **WX-02 fog tide:** a third of the L1 cells let the fog thicken by up to 70 % in banks that drift
  with a period of 480 s (a triangle wave of the clock and `x + z`).
- **WX-03 rain** (1 L1 cell in 14, in a Between, radius 600 to 1400 m), **WX-04 dust fall** (1 in
  10, radius 500 to 1200), **WX-05 ash snow** (1 in 30, radius 900 to 1500): fields of particles that
  slide with their velocity: rain 11 m/s down, dust 0.4 m/s, ash 0.9 m/s and drifting.
- **WX-07 wind:** a sixth of the L1 cells have a wind of 3 to 10 m/s along one axis; dust blows
  with it (the dust field moves along the wind). *Deviation:* the wind is by cell, not confined
  to bores and Roads.
- **WX-08 frost:** blobs of 400 to 900 m (1 in 16): pale fog and slow rime. *Deviation:* no rime
  on railings and no visible breath.
- **WX-09 heat haze:** blobs (1 in 16) and the furnaces of wells (within 700 m): warmer, thicker fog.
  *Deviation:* no shimmer of the image.
- **WX-10 lightning:** inside a fog sea, one 8 s slot in three holds a stroke of 400 ms within an L1
  cell; a zigzag of beams and a white light that fades.
- **LGT-05 window weather:** the module glow is multiplied by a wave of 0.55 to 1 that crosses
  with the clock and position (period 240 s).
- **LGT-07 furnace glow:** a warm point light at each furnace within 400 m, lighting the fog from
  below.

Engine: a field is a block of 96 × 48 × 96 m of static particles drawn 3 × 3 × 3 times and slid by
chunk origin, so no frame uploads anything; it shows when the effect is above 0.1 and its
chunks are made the first time. `SUPERSTRUCT_WEATHER=rain|dust|ash|frost|heat|wind|fog` forces an
effect for captures.

### 5W.48 Census v2 (PW.13.1, `worldgen::defects_audit`; no `GEN_VERSION` change)

Counts the fault classes of the polish audit (`docs/playtests/2026-10-22-polish-audit.md`) over a
list of L2 cells with their L3 interiors and parent L1 pieces. `web-report --defects N --cells FILE`
runs it after the PW.4.8 census (`N` may be 0 with `--cells`; the list is one `x,y,z` per line).
Every count is keyed `"<letter> <what>"`; the fixed list is `docs/playtests/2026-10-22-audit/cells.txt`.

- **A** walk clearance: samples along every leg and rise against every solid (L1 except inside bores,
  final L2, L3): blocked at 0.5 and 1.2 m, headroom at 1.8 m, no floor within 0.6 m below and 0.3 m
  sideways. By leg or rise, by blocker and by path id. Skipped in tilted lattices.
- **B** clusters of solid (and cables) that touch nothing, joined at 0.15 m, judged on pieces, at least
  1 m from the cell's faces and not touching L1: under 2 m, under 10 m, larger.
- **C** interiors (the masses of `l3_web::mass_specs`) sampled every 3 m against corridors, bores,
  trams, other L2 solid and L1 solid; masses with a shell face missing.
- **D** two legs on one line: parallel, within 0.6 m, sharing at least 4 m.
- **E** solid pieces thinner than 0.1 m, and crumbs under 0.02 m³.
- **F** walk lines (at 0.6 and 1.4 m) through non-solid decoration.
- **G** non-solid plates of at least 1 × 1 m and at most 0.6 m thick (not windows, glass, voids).
- **H** pieces of 16 m or more that the mid band does not draw (the full band skips `MID_ONLY`).
- **L** L3 solid whose middle is more than 0.6 m outside every mass.
- **R** rails with no walk line within 2.5 m + half the width, and rails whose middle lies in solid
  of a path or threshold (rails inside an enclosed path).
- **S** structure parts sliced by a cell face, by upright or turned lattice.

The PW.4.8 census (`worldgen::defects`) is corrected in four places: counter `g` no longer counts
`MID_ONLY` boxes as drawn; `f air` accepts a leg ending on the side of another leg (T junction);
the example cap in `Report::merge` keys on the counter's first two words, not the id; counter `b`
is superseded by A for clearance and kept only as a witness of path pieces in walk space.


## 6. L3: Blocks (64 m)

> **Web redesign.** From version 7 the plan, entrances and skeleton are those of §6W. The rest of this section (tiles, solver, sockets, fallback, validator) is reused.

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
- **Extra links (loops).** For each node `i` in order, its nearest other node `j` that no tree edge joins to `i` (same distance and tie rules) is a **candidate**, unless `(i, j)` is already a candidate. (As first written, "unless `(i, j)` is a tree edge" left almost no candidates, since a node's nearest neighbour is nearly always a tree edge; corrected 2026-10-18 in P4.1.6.) Each candidate draws `chance(0.3)` from `hash(cell, WALKWAY)`, in candidate order.
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

## 6W. L3 in the Web: buildings as passages (64 m)

*PW.0.5. Version 7. §6 stays the specification of versions 1–6 and of the tile solver, boundary sockets, fallback and validator, which are reused. This section changes what feeds them: any number of entrances on any face or storey, a skeleton over all of them, and interior archetypes. Owners: L3-01..21 (PW.4.1–PW.4.3), INT-01..19 (PW.4.4, PW.4.5). Architecture: `the-web.md` §10.1.*

### 6W.1 Mass lattice and frames

- A **mass** is an L2 structure (`§5W.4`, `ENVELOPE`). It carries a frame `F` (`regimes.md` §7.4) with **local up** `ey`. Its floors are always normal to `ey`: inverted, hanging, sideways, tilted and radial forms differ only in silhouette, support and entrances (L3-21).
- Its **mass lattice** is the lattice of `§6.3` (8 × 16 × 8 slots of 8 × 4 × 8 m per block, storeys along `ey`) laid in the frame `F`, with its origin at the mass's frame origin.
  - For `Id` and quarter-turn frames (the frame maps world axes to world axes) the mass lattice is the **world L3 lattice**: blocks are the world L3 cells, exactly as in §6.
  - For any other frame, **mass blocks** are the 64 m cubes of the mass lattice. A world L3 cell does not solve them; it emits, for every mass block it overlaps, that block's pieces **clipped to the L3 cell box** (§1.4). A mass block is a pure function of `(mass key, block index)`, so every world cell that touches it derives the same block. Mass blocks are cached by that key.
- The **mass key** is `hash(seed, L2, owner, L3_MASS)` with the mass index absorbed; a mass block adds its lattice index. Everything below that says "block" means a world L3 cell or a mass block according to the frame.
- A block is **active** if it overlaps an envelope of its mass with positive volume (test on the integer AABB, `primitives.md` §9.5, then on the oriented volume).
- **Twisted forms** (L3-13): the form has a central core column (a spine) that is rotationally symmetric. Every storey has its own lattice frame, turned by `k · δ` headings (`δ` from the form), and storeys connect **only** through the core: vertical sockets are P on the core column and F elsewhere. Each storey's skeleton is then solved in its own frame, and the core column's door is on the storey's own ring.

### 6W.2 Entrances

*Owners: L3-01, L3-02.*

An **entrance** is an `Entrance` port (`ports.md` §2.1) with a `ForcedPort` into L3. A mass has **any number** of them: `0 ..` as many as L2 emitted (§5W.2). Where they come from:

- the structure's support contact (§5W.2 step 3.6), one or more;
- every end of an engulfed route (§5W.2), at the storey given by the route's height;
- additional entrances for large masses: one per 30 m of footprint, up to 8;
- L2 set pieces and thresholds on the mass.

```rust
pub enum EntranceKind {
    Door,      // a doorway in a side wall, any storey
    RoofHatch, // opening in the roof, to the top storey
    FloorHatch,// opening in the floor of storey 0 (hanging forms, structures on stilts)
    Ladder,    // a ladder port: a hatch with a ladder well of one column
}
```

- Each entrance has a **storey** `t_e` and a **column** `c_e`:
  - `Door`: `t_e` is the storey whose floor height is nearest the entrance's floor (the route's height). `c_e` is the IN column behind the doorway, found as in §6.2 (apron, middle 4 m). If the doorway is not within an IN column, the nearest IN column in the facing direction is used and the rind between is cut as the passage of §6.4 step 4.
  - `RoofHatch`: `t_e` = the top storey, `c_e` = the IN column under the hatch.
  - `FloorHatch`, `Ladder` at the bottom: `t_e = 0`.
  - `Ladder`: the same as the matching hatch, but the vertical opening is a 1 × 1 m ladder well (`Ladder` tile, `CLB` rules, `the-web.md` §9) rather than a stair.
- The entrance doorway is at least 4 m wide for `Door` and 1.5 × 1.5 m for hatches; both obey the minimum clearance of `ports.md` §2.
- Entrances are numbered in the order the L2 cell emitted them (`ports.md` §2.1), and a mass's entrance list is the same in every block that meets it.
- An entrance whose column is not found (the mass has fewer than one IN column, a "vestibule-only" or sealed mass) makes the plan **Vestibule-only** or **Sealed** as in §6.4, with the count of entrances ≥ 1 forcing at least Vestibule-only.

### 6W.3 Plan and skeleton over all entrances

*Owners: L3-03.*

Every block that meets a mass computes the same plan from the parent output alone, using `Rng::new(s_i)` of §6.4 where the draw order is extended. It generalises §6.4 as follows.

**Plan class.** *Sealed* (no entrances, or `NO_ENTRANCE`), *Vestibule-only* (exactly one IN column), *Full* (two or more).

**Full plan.**
1. **Vestibules.** For each entrance `e`, the slot `(c_e, t_e)` is a fixed `Vestibule` (a hatch variant for `RoofHatch`, `FloorHatch` and `Ladder`). Two entrances may share a slot (the same column and storey): the slot then has both doorways.
2. **Spines.** The number of spines is `n_s = min(4, 1 + floor(|IN| / 16))`. Each spine `s_j` is drawn in turn, with `below(n)` among the IN columns that are not already a spine or a vestibule column and **whose removal, together with the earlier spines, keeps the IN set 4-connected** (for the first spine, the candidate rule of §6.4). If there is no candidate, the spine count is reduced. The door direction of `s_j` is drawn among the directions whose neighbour is IN and not a spine. Every spine occupies every storey of its column, as in §6.4. For twisted forms the spines are the core column only.
3. **Storey trees.** Each storey `t` draws `below(24)` (a permutation of the four directions). The **targets** at `t` are:
   - the door neighbour `d_j` of every spine `s_j`;
   - the column `c_e` of every entrance with `t_e = t`;
   - for every 4-connected component of `IN ∖ spines` in a block part, the component's column nearest the first spine's `d` (ties: lower z, then lower x).

   A BFS from `d_1` over `IN ∖ spines`, expanding in the drawn order, gives a tree; the **skeleton slots** at `t` are the union of its paths from `d_1` to all targets. The tree is connected because `IN ∖ spines` is 4-connected by the spine rule.
4. **Entrance passages.** The rind between each doorway and its vestibule is solid minus a passage of the doorway's width, 3 m high, with a 4 Q floor slab (as §6.4 step 4). Hatch passages are vertical: the roof or floor slab between the outside and the vestibule is cut by the hatch.
5. The **skeleton** `K` is the spines, the vestibules and the tree slots.

The plan is a pure function of the parent output, so blocks agree without communicating. **Entrances at any storey** need no special cases: a storey with no entrance has a tree to its spine doors only.

**Why this makes buildings through-routes.** Every entrance column is a target of the tree at its storey, and every storey's tree reaches a spine door, and spines join all storeys. So `K` is connected and contains every entrance (Lemma L3W-1 below). A building with two or more entrances is therefore a through-route between all of them. Which routes exist through it is the interior's variety.

### 6W.4 Archetypes

*Owners: L3-04..13 (PW.4.2).*

An **archetype** adapts the interior to the form. It never changes the skeleton or the boundary conditions; it only changes tile weights, restricts non-skeleton slots, and fixes extra non-skeleton content.

```rust
pub struct Archetype {
    pub id: ArchetypeId,                   // L3-04..13 or INT-xx
    pub forms: &'static [FormFamily],      // where it applies
    pub weights: TileWeightDelta,          // multiplies tile weights of the solver
    pub fixed: fn(&MassPlan) -> Vec<FixedSlot>, // extra fixed slots that are not in K
    pub payoff: bool,                      // whether it provides an interior payoff (§6W.6)
}
```

- The archetype is picked per mass by `pick_weighted` over those that apply to the form and are not banned by the character, weighted by the character's quarter weights (`regimes.md` §10).
- A fixed slot added by an archetype must not occupy a skeleton slot, must keep every socket consistent (the same rule as §6.6), and must keep the acceptance rule satisfiable (a fixed slot that is walkable must be adjacent, by a D side, to a skeleton slot or another walkable fixed slot that is).
- The list (L3-04..13):

| Id | Archetype | Applies to | Restriction |
|---|---|---|---|
| L3-04 | office grid | slabs, towers | regular W/D pattern at 2-slot pitch; corridors on the skeleton |
| L3-05 | cell block | slabs, drums | rows of 1-slot cells off a skeleton corridor |
| L3-06 | hall with mezzanine | towers, cages | a double-height `Atrium` core with an edge gallery |
| L3-07 | ramp core | towers, drums | the spine is replaced by a helical ramp (walkable slope) |
| L3-08 | stacked lofts | slabs, sideways | alternating wide storeys |
| L3-09 | open cage | cages, gantries | most slots `Out`-like, floors only on the skeleton |
| L3-10 | machine floors | plants, machines | big `Atrium` volumes with catwalks on the skeleton |
| L3-11 | rack aisles | rack canyons, data | long narrow aisles between solid racks |
| L3-12 | radial plan | rotundas, domes | spokes from a central spine |
| L3-13 | twisted floors | twisted towers | per-storey frame (§6W.1) |

The rule "**banning** ≥ one archetype never leaves none": the plain solver (no archetype) is always legal and is the fallback.

### 6W.5 Corridor kinds and interiors

*Owners: L3-14..20 (PW.4.3), INT-01..19 (PW.4.4, PW.4.5).*

- **Corridor kinds** are tile classes and restrictions applied along skeleton edges: galleries over double-height rooms (L3-14), stepped corridors that climb half a storey (L3-15, a 2 m rise within one slot: allowed because the step-up total stays within the ramp rule, `traversal.md`), enfilades of rooms in a line (L3-16), dog-legs with a window at the bend (L3-17), corridors open to an atrium (L3-18), skew corridors at the grain angle (L3-19, an oriented corridor slab clipped to the block, built from the same `Oriented` boxes), and ladder wells between floors (L3-20, `Ladder` tiles). Each is drawn per skeleton edge by `pick_weighted` over the character's weights; a kind that cannot fit is skipped, down to the plain corridor.
- **Interiors batch 2** (INT-01..19) are specified as **set-piece rooms** with a footprint in slots and a restriction list, placed by the archetype step on non-skeleton slots, with each fixed slot respecting the rule of §6W.4. They provide payoffs (§6W.6).
- All of them are built from the existing tile alphabet plus the new tiles `Ladder`, `Hatch`, `Ramp` (a half-storey ramp), `Gallery` (a floor with a drop), and `Rack` (solid with aisles). New tiles are **appended** to the alphabet (`§6.5`), with sockets that follow the rules there, and each new tile adds its masks.

### 6W.6 Payoffs inside buildings

A mass with one entrance is a dead end (PRN-05) unless it is `Sealed`. Its payoff is inside:

1. an archetype's **payoff room** (INT-xx, a window room onto a long sightline, a lift lobby, a shrine) placed at the walk-farthest walkable slot from the entrance;
2. else a **view room**: the farthest room that has an exterior face gets a window slot (an opening in the rind) facing a free or void direction;
3. else a **lamp** in the farthest room (always possible).

A mass with ≥ 2 entrances needs none: it is a through-route.

### 6W.7 Boundary conditions and agreement

Section 6.6 stands with these changes:
- Block faces mean **mass block faces** (the mass lattice). Face patterns are drawn from `hash(seed, L3, mass block owner, PORT_FACE_X | PORT_FACE_Z)` with the mass key absorbed first, so two blocks of the same mass agree. The pattern is not used across a boundary between two different masses (masses are ≥ 8 m apart, so no `Core` slot touches another mass's).
- A `Vestibule`, spine, hatch and ladder slot is a **fixed slot**; its sockets are fixed.
- For clipped (non-aligned) mass blocks the L3 ports on the **world** L3 cell faces are not generated: the world cell has no solve. The ports of a mass block are those on its own faces, and the world L3 cell lists the entrances that lie in it, as before.

### 6W.8 Acceptance and the fallback

Section 6.8 stands. The fallback rooms set R now includes every vestibule and every spine door, and the fallback path from each face-D slot ends at the nearest skeleton slot (which now includes all vestibules). The proofs are rewritten for the plan above.

**Lemma L3W-1 (K connects every entrance).** In a Full mass, `K` is connected in the walk graph across blocks and contains every entrance's vestibule. *Proof.* Each storey's tree is a connected BFS tree over `IN ∖ spines` (the spine rule keeps that set 4-connected), containing every target, which includes the door neighbours of all spines and the column of every entrance at that storey. Tree edges are O/D inside a block and fixed D across block faces, the same in both blocks. Every spine door is a skeleton slot at every storey, and a spine's P stack joins all storeys. So any two skeleton slots are walk-connected, and every vestibule is a skeleton slot. ∎

**Lemma L3W-2 (block closure) and L3W-3 (geometry).** As Lemmas L3-3 and L3-4, with `K` as defined here. Archetype fixed slots obey §6W.4, so the acceptance rule and the fallback still apply. The fallback is legal for every boundary condition because it uses only `Room` and `Solid`, plus fixed slots, whose sockets are fixed.

**Theorem L3W (through-routes).** For every mass with entrances `E` (|E| ≥ 1):
1. every entrance connects, inside the parent L2 cell, to N (Theorem L3, via its apron, hatch landing or ladder base, which L2 joined to its routes, §5W.2 step 3.6);
2. for |E| ≥ 2, every entrance is walk-connected to every other entrance **inside the mass**;
3. every L3 walkable surface of the mass is connected to `K`, or sealed.
*Proof.* (1) is Theorem L3 with the new `K`. (2) is Lemma L3W-1. (3) is the acceptance rule. ∎ "Inside the mass" is within the union of its blocks, each of which attaches to `K` by Lemma L3W-2.

### 6W.9 Salts, draw order and census

- New salts (L3 block, `0x0400–0x04FF`): `L3_ARCH` `0x0410` (archetype pick), `L3_CORR` `0x0411` (corridor kinds), `L3_PAYOFF` `0x0412` (payoff rooms). The existing `L3_MASS`, `L3_SOLVE`, `L3_RETRY`, `L3_PIPE` and the port face salts are reused.
- Draw order in a mass plan: vestibules (no draws), spines (`n_s` draws), the door directions, `below(24)` per storey, archetype, corridor kinds per skeleton edge in index order, payoff.
- The census counts, per mass, the archetype id, the corridor kinds, the entrance kinds and the payoff id (`Instance` tags, §5W.9).
### 6W.10 As implemented: masses, entrances and plans (PW.4.1, `worldgen::l3_web`, `l3_plan`, version 17)

- **A mass is a box part of a structure** (`Structure::parts`, §5W.12), on the L2 cell's lattice, whose frame is the cell's lattice frame (so a mass is aligned with its own blocks, as §6W.1 asks, and the solve of §6 runs in lattice coordinates). A part that shares volume with an earlier mass (a fused neighbour, MUT-14) is not a mass and stays solid, and so does a part that no entrance leads into (a sealed mass: no interior, the L2 box stays whole). Shaped, turned and hanging pieces of a structure are not masses.
- **Blocks** are the 64 m cubes of the lattice (8 × 8 × 8 of them in an L2 cell). `L3` cell keys inside an L2 cell are lattice block indices (`l2 · 8 + index`). The engine's chunks are the world L3 cells: `WebL3::cell_primitives` solves every block a world cell meets and returns its pieces turned into place by the lattice frame (as `Oriented` shapes with the block's origin in the cell's local quanta) and clipped to the cell's box. Blocks are cached per L2 cell context.
- **Entrances** come from `Structure::entrances` (the route slot outside and the direction into the structure, §5W.12), each given to the part the slot beside the route slot belongs to: a **door** on that face (4 m wide, centred on the slot, so half in each of the two middle columns of its four, at the storey of the slot's floor), a **floor hatch** (a route below the part: a 4 m square opening in the floor of the bottom storey, in the middle column pair's second column) or a **roof hatch** (a route above: an opening in the roof of the top storey, over a ladder well of rails and rungs that do not collide; walking down needs the ladder mechanic of PW.5.1). Two touching parts of one structure are joined by a door on each side at the lowest first contact cell. Vertical contacts are not joined.
- **The plan** (`MassPlan`, §6W.3): a vestibule on every entrance column and storey (a door's vestibule is two columns); `min(4, 1 + columns/16)` spines (spiral stairs through every storey), each in a column that is not a vestibule column and keeps the others 4-connected; per storey a BFS tree from the first spine's door neighbour over the columns that are not spines, whose paths reach every spine's door neighbour, every entrance column at that storey and the nearest column of each 8 × 8 block part; the skeleton is the union. Plan classes are Sealed (no entrance), Vestibule-only (one column) and Full. The vestibule tile is walkable and links to any open or door side of a neighbour.
- **The L2 side:** a mass is a hollow shell 0.5 m thick (six slabs, flag `INTERIOR`, still `ENVELOPE | FAR_VISIBLE`) with its doors (3 m high) and hatches cut, instead of a solid box. The shell collides; L3 collides inside it.
- **Not done in this item:** the L4 modules for the Web blocks (their placements are in block-local axes, not in a tilted frame), the L3 validator of §6.14 (replaced here by `every_entrance_reaches_every_other`), twisted floors (L3-13 is PW.4.2).
- **Tests.** `every_entrance_reaches_every_other`: in 120 cells (3 900 masses, 2 960 with more than one entrance, 13 000 entrances) the vestibules of all entrances of every Full mass lie in one walk component of the solved tiles; `world_cells_hold_their_interiors`: the pieces stay in their world cells and two builds agree.

### 6W.11 As implemented: archetypes (PW.4.2, `worldgen::l3_arch`, version 17)

- **The pick.** `l3_arch::pick` draws the archetype of mass `i` of an L2 cell with salt `L3_ARCH` (keyed by the cell plus the index), by `pick_weighted` over `applicable(form)`: the plain solver always (weight 3) and the archetypes the structure's form suits (`Structure::form`: slabs, towers and pairs take L3-04 office grid, L3-05 cell block, L3-06 hall with mezzanine, L3-07 ramp core, L3-08 lofts; the ziggurat offices, lofts and halls; cages and gantries L3-09 open cage; machines and tank farms L3-10 machine floors; the rack canyon L3-11 rack aisles; drums, rotundas and domes L3-12 radial plan, ramp cores and cell blocks; the twisted tower L3-13; leaning, angled and wedge forms lofts and offices; hubs halls, machine floors, racks and radial plans; sets and the rest an office or a hall). The character's quarter weights of §6W.4 are not read (uniform within the table).
- **What an archetype is.** A multiplier, in eighths, on every tile weight of the solver at every slot of its mass (`l3_arch::weight_mult`, applied by `l3_solver::solve_weighted`), by tile family, count of open and door sides and storey parity (lofts); never a change of tiles, sockets, skeleton or boundary conditions, so acceptance and the fallback stand. Office grid: closed rooms with one door ×8; cell block: one-door cells ×15, corridors ×5; hall with mezzanine: atria ×12, voids ×2.5; ramp core: one spine more (at most four); lofts: even storeys open (rooms with three or more open sides ×5, atria ×3), odd ones partitioned; open cage: voids ×20, solids ×1/8; machine floors: atria ×15, voids ×5; rack aisles: solid racks ×5 and straight corridors ×15; radial plan and twisted floors change the tree (below).
- **Radial plan (L3-12) and twisted floors (L3-13)** change the BFS tree of every storey: it grows first along spokes (the row and the column through the first spine; for twisted floors the one ray that turns a quarter every storey, so the corridors of a tower turn about its core) and then over the rest, so it still reaches every target. **Deviation (decide):** the registry's twisted floors turn each storey's whole lattice frame; a frame of a mass is one `Table` frame and a storey cannot be turned in a tilted lattice, so the floors' corridors turn and the walls do not.
- **Census.** `MassPlan::archetype`; in 120 cells (3 600 masses) all eleven occur (plain 1 100, mezzanine 650, open cage 500, office 370, cell block 280, lofts 210, ramp core 200, radial 140, machine floors 110, rack aisles 70, twisted 30), and the signatures hold: open-cage interiors are 13 % void against 5 % in plain ones, machine floors 9 % atria against 3 %, rack aisles 33 % solid against 19 % (`every_entrance_reaches_every_other`).

### 6W.12 As implemented: corridor kinds (PW.4.3, `worldgen::l3_dress`, version 17)

- Drawn after the solve (`L3_CORR`), one draw per slot in index order, to one skeleton slot in four with a kind it can take, chosen by `pick_weighted`; the rest stay plain corridors. They add pieces and, for ladders, holes; they never change a tile. Collidable additions stand only where a passage keeps its width and a head clearance of 2 m.
- **L3-14 gallery:** a rail 1.125 m high along a skeleton slot's open side that looks onto a void, or onto an open room whose floor is the slot's double height; a 4 m gap in the middle keeps the way into the room. **L3-18 corridor open to an atrium:** the same rail on an open side onto an atrium of the same storey. **L3-15 stepped corridor:** a hump on a straight corridor: four steps up (0.75 m, 0.1875 m by 0.3125 m), a landing, four steps down, inside the 2 m passage; (the registry's 2 m rise does not fit a 3 m ceiling). **L3-16 enfilade:** a lintel across the doorway at the end of the middle slot of three straight slots in a line. **L3-17 dog-leg:** a lit window pane on the inner face of an outside wall at the bend (two skeleton edges at a right angle). **L3-19 skew corridor:** a chicane of diagonal teeth 0.125 to 0.25 m deep along the corridor's walls, within 0.25 m of them (a turned corridor slab cannot be made in the lattice's axes: stand-in, decide). **L3-20 ladder well:** a room with a room above gets a 1 m hole in the floor above (the slab becomes four boxes round it) and a ladder of rails and rungs that do not collide, one in sixteen eligible rooms; climbing it is PW.5.1.
- **Census** (`every_corridor_kind_occurs`, 40 cells): all seven occur (`L3Layout::corridors`).

### 6W.13 As implemented: interior set pieces (PW.4.4, PW.4.5, `worldgen::l3_rooms`, version 17)

- INT-01 room of stairs, INT-02 hall of doors, INT-03 cubicle field, INT-04 parking (bays, cars and a ramp; the helix is not built), INT-05 lift lobby of forty (ten doors on each wall), INT-06 ward (sixteen beds a slot, a hundred over a ward's slots), INT-07 auditorium (tiered seats), INT-08 stadium hall (terraces), INT-09 server cathedral (racks that line up from storey to storey: the kind and the corners are drawn per column and storey group), INT-10 cistern of pillars, INT-11 bunk hall, INT-12 the Vessel (plates of cylinders), INT-13 shrinking corridor (teeth that grow along the corridor, leaving 1 m), INT-14 dining hall, INT-15 map room (a plinth with a relief), INT-16 glass room, INT-17 clock room, INT-18 shower hall, laundry hall and kitchen (stalls and pipes), INT-19 mirror corridor.
- **Placement.** One eligible slot in twelve (`L3_PAYOFF`): rooms and atria off the skeleton take a room kind (atria: stadium, server hall, cistern, vessel, parking, auditorium; rooms the others), straight corridors on the skeleton take INT-13 or INT-19. Collidable pieces stand only in the four corners of the slot, clear of a middle cross 1.5 m wide, so every door of a slot still joins every other and acceptance holds (test `rooms_leave_the_middle_of_a_slot_clear`); frames, panes and lamps do not collide.
- **Deviations (decide):** the interiors are decoration of the solver's open slots, not new tiles (the alphabet of §6W.5 gets no `Ramp`, `Gallery` or `Rack` tile), so a room is at most one slot big and the parking helix, the hundred-bed ward and the thirty-metre racks are repeated slots. Payoff rooms (§6W.6) are not placed: through-routes need none and a mass with one entrance is a dead end only when its structure is.
- **Census** (`every_room_occurs`, 30 cells): all nineteen occur (`L3Layout::rooms`).


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
- **As implemented (P4.2.2, `worldgen::l4`).** `block_modules(seed, block, &L3Block)` returns plain `Module` placements in block-local quanta; the engine builds the meshes (`engine::kit`). Per walkable `Core` slot: a `WallPanel` on each W side (variant from a fixed draw of 8 per L4 cell, `hash(L4 cell, MODULE)`: 0 plain 50 %, 1 ribbed 25 %, 2 vent 12.5 %, 3 window 12.5 %), a `DoorFrame` on each D side at the floor top, a `Grating` overlay on `Corridor`, `SpiralBase` and `SpiralTop` slots, a `Lamp` on the ceiling of `Room`, `Corridor` and vestibule slots. One `Pipe` module per 8 m of every L3 pipe cylinder and one `Cable` bundle beside the first pipe of each run. `railing_modules` turns the railing boxes of an L2 cell into `Railing` modules, one per box, so guarded runs get one and unguarded runs none (`l2_rail::railings_with_runs` exposes the runs and decisions). Every module is `NO_COLLIDE` here: the box under a railing or pipe keeps carrying collision. Not yet placed: atrium balustrades, pipe bends, facade windows and emissive panels on the outside of rinds, lamps in atria and spirals.
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
| `TRAM_LINE` | `0x0209` | `LayerKey(L1, W = 4)` | tram lines per deck (`trams.md` §2.1) |
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
