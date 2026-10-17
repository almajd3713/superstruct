# Spec: ports and connectivity

**Status: approved by the user on 2026-10-17 (P0.2.6).**

Ports are the openings where walkable space crosses a cell face. They are how neighbouring cells agree on connectivity without communicating (CLAUDE.md §3.4–3.5). Geometry follows `primitives.md`, keys follow `levels.md` §1 and hashing follows `hashing.md`.

## 1. Faces and `FaceKey`

- The face between cell `c` and `c + e_axis` is **owned by the lower cell** `c`: `FaceKey { level, owner: c, axis }`.
- A cell's `+axis` face has owner `c`. Its `−axis` face has owner `c − e_axis`, which is a checked operation (`coordinates.md` §7).
- **Face plane coordinates `(u, v)`** are measured in Q from the face's minimum corner. The two cells sharing a face have identical tangential coordinates, so they read `(u, v)` identically:

  | Axis | u | v |
  |---|---|---|
  | X | z | y |
  | Y | x | z |
  | Z | x | y |

- In the owner's local space the face lies at `axis = size`. In the neighbour's local space it lies at `axis = 0`.

## 2. The `Port` struct

```rust
pub struct Port {
    pub face: FaceKey,
    pub id: u16,          // index on this face, deterministic order
    pub kind: PortKind,
    pub rect: QRect,      // opening in face-plane (u, v), half-open
}

pub enum PortKind {
    Walkway,          // X/Z faces: a walkway or skywalk crossing
    DeckContinuation, // X/Z faces: an L1 deck crossing at the same height on both sides
    BridgeEnd,        // X/Z faces: an L1 bridge crossing
    StairLanding,     // Y faces: a stair passage
    LiftStop,         // Y faces: a lift core passage; also internal stops (not on faces)
}
```

**Geometry rules**

- **X/Z faces.** `rect.v0` is the **floor height**. Walkable floor on both sides meets the face flush at exactly `v0` across `[u0, u1)`, and the opening is clear of collidable solid up to `v1`.
- **Y faces.** `rect` is the passage footprint. The lift car or stair run passes through it.
- **Minimum sizes,** derived from the capsule in `traversal.md`:
  - Clearance `v1 − v0` is at least 2.4 m (1.8 m capsule plus 0.6 m headroom).
  - `Walkway` is at least 2 m wide.
  - `BridgeEnd` is 8–32 m wide.
  - `DeckContinuation` spans the shared deck along the whole face.
  - `LiftStop` and `StairLanding` are at least 8 × 8 m.
- **Margins.** Ports keep 64 m from face edges at L1 and 8 m at L2, so a port never touches an edge or corner shared with a third cell.

## 3. Deriving ports

A face's ports come from two inputs only:

1. **The face key.** An RNG is seeded with `hash(seed, level, owner, PORT_FACE_{X|Y|Z})`.
2. **The face context.** This is the **keyed features** of both adjacent cells: decks from layer keys, trunks, shafts and the lift core from column keys, chasms from the cell hash, and the regime and cavity from L0. These are pure functions of keys and are cheap to evaluate (`levels.md` §4.0). Port derivation never reads either cell's generated output, so both cells compute the same ports.

Ports are produced in a fixed order and numbered `id = 0, 1, …`. A port whose rect would violate the face context, for example a floor inside a deck slab on either side, is not moved. It is dropped, and the draw count stays fixed so later ports are unaffected.

## 4. Port types per level

### 4.1 L1 faces

| Face | Ports |
|---|---|
| X/Z | **DeckContinuation** wherever both sides have a deck at the same height. Inside a 16 km deck region that is always the case. **BridgeEnd:** 1 + `below(3)` bridges, so 1–3 per face. Each floor height `v0` is drawn on a 4 m step and redrawn up to 8 times until it is at least 8 m clear of every deck slab on both sides. After 8 failures it falls back to the midpoint between two adjacent decks, which is always clear because decks are at least 600 m apart. `u` is drawn with a 64 m margin |
| Y | **LiftStop**: the passage of the column's lift core (`levels.md` §4.0) |
| any face touching a cavity cell | none |

The result is that **every face between two non-cavity L1 cells carries at least one port.** At L1, the backbone lattice is the whole grid (§6).

### 4.2 L2 faces

| Source | Ports |
|---|---|
| **Cascade** (§5) | Every L1 walkable structure that crosses the L2 face gives a port of the matching kind: deck surface, bridge deck, lift core |
| **Own ports** `[PORT_FACE_*]` | 0–2 **Walkway** (skywalk) ports per X/Z face, with `chance(lerp(0.2, 0.6, d))` each. Allowed only where both sides have the **same L1 deck** under the face. The floor is that deck's top plus 4 m × `range(0, 8)`, one of the first 8 floors |

L2 has no own ports on Y faces. Vertical movement inside an L1 cell uses L2 towers, which are internal, and the L1 core.

### 4.3 L3 and finer

L2 ports and envelope entrances become L3 boundary conditions (P4.1.1). The same rules apply.

## 5. The cascade (L1 → L2 → L3)

- L1 walkable structure (deck tops, bridge decks, the lift core) is **structural**. Finer levels never remove it, and they keep its corridors open with `KeepOut` constraints over bridge decks, core landings and the region 2.4 m above deck walkways.
- Where such a structure crosses an L2 face, both L2 cells see a **forced port** at the intersection. Both compute it from the same L1 keyed features. The same mechanism carries L2 ports into L3.
- **Finer levels add collidable solid only** inside their parent's envelopes, on their own walkways and as railings along edges. Anything else they add on L1 walkable surfaces is `NO_COLLIDE`. This is what keeps the L1 network unobstructed.

## 6. Connectivity argument

**Definitions**
- A *cavity cell* is an L1 cell inside a giant cavity.
- The **network N** is the union of L1 walkable structure (deck tops, bridge decks, lift-core stops) over all non-cavity L1 cells.

**Lemma A (L1 local connectivity).** In every non-cavity L1 cell, all L1 walkable structure and all of the cell's L1 ports lie in a single walkable component, using L1 geometry only. This holds by construction (`levels.md` §4):

1. Every deck in the cell is connected. Holes (shafts, chasms, the core) and trunk footprints each keep at least 64 m from each other and from the faces, so a deck minus its holes is a connected region.
2. The lift core has a stop on every deck top in the cell. That connects all decks.
3. Every bridge runs from its BridgeEnd port to a core stop at its own height. If no axis-aligned route clears the trunk, the trunk segment is omitted in this cell. That fallback is deterministic, counted in stats, and expected to be rare.
4. Y-face LiftStop ports are the core itself.
5. DeckContinuation ports lie on decks, which are connected by (1).

By §5, finer levels do not obstruct any of this.

**Lemma B (agreement).** Both cells compute each port identically (§3). On both sides the walkable floor meets the port rect flush at `v0` (§2), so a port joins the two cells' components.

**Lemma C (grid connectivity).**
- Every face between two non-cavity L1 cells carries at least one port (§4.1).
- Cavity cells form boxes aligned to the L1 grid, with at least one L1 cell of margin inside their L0 cell (`levels.md` §3), so two cavities never touch.
- The cubic grid ℤ³ minus a set of pairwise non-touching finite boxes is connected: any path can step around a box through the layer of cells next to it.
- So the graph of non-cavity L1 cells joined by ported faces is connected.

**Theorem (global connectivity).** N is connected.
*Proof:* by A, each non-cavity L1 cell's part of N, together with its ports, is one component. By B, adjacent cells' components merge through every shared port. By C, these merges connect all non-cavity cells. ∎

**Lemma D (finer levels attach to N).** In every L2 cell, each port and each walkable surface the cell generates (masses' entrances, walkways, stairs, towers) connects **within the cell** to N. This is guaranteed by `levels.md` §5:
- masses sit only on deck tops,
- own skywalk ports exist only above a deck on both sides,
- the walkway solver links everything to the deck or core it stands on,
- the validator (P0.9) rejects any cell that fails, as a generator bug.

L3 has the same obligation toward L2 surfaces. Deliberately sealed rooms and dead ends are marked as such and are exempt.

**Corollary (horizon promise, reachability).** Every walkable surface that is not marked sealed is reachable from every other one. Every far-visible structure (deck, bridge, trunk) can be reached by walking:
- decks and bridges are part of N,
- a trunk segment passes through a deck in its cell, or, in cells with no deck, the bridge router gives one bridge a landing against the trunk (`levels.md` §4.5).

**What this does not claim.** The argument does not cover the time it takes. A 20 km walk is reachable but slow, which is why transport (lifts, trams) exists. It assumes Lemma A's construction is implemented correctly; P0.9.4 and the phase-exit check over 10⁶ cells (P0.11.5) test exactly that. Gravity regions (P5.2.4) would need this argument revisited.

## 7. Salts

| Salt | Value | Key | Use |
|---|---|---|---|
| `PORT_FACE_X` | `0x0F01` | FaceKey, axis X (all levels) | ports on X faces |
| `PORT_FACE_Y` | `0x0F02` | FaceKey, axis Y | ports on Y faces |
| `PORT_FACE_Z` | `0x0F03` | FaceKey, axis Z | ports on Z faces |

The level is part of the hash, so one salt per axis serves every level without collision.

## 8. Review decisions and open questions

**Decided (2026-10-17)**
- **Local rule at L2 and finer:** each port connects to the L1 network N inside the cell (Lemma D). CLAUDE.md §3.5 has been reworded to match.
- **Lift core:** one in every L1 column (`levels.md` §4.0.1). Approved.

- **Backbone at L1:** every face between two non-cavity L1 cells carries at least one port (§4.1). The user chose this over PLAN.md's sparse lattice: islands at 4 km scale would break the horizon promise, and local variety comes from L2 and L3 dead ends, empty deck layers, shafts and chasms.
