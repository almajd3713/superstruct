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
| `ColumnKey` | `level, x, z` | `hash(seed, level, (x, 0, z), salt)` | Vertical features: trunk position and radius, lift shafts, stair towers |
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
