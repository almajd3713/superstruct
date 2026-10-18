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
    LiftStop,         // Y faces: a lift core passage
}
```

Every `Port` has a `FaceKey`: ports exist only on faces. A lift core's internal stops (one per deck or floor inside an L1 cell) are not `Port`s. The engine derives them from the generator's keyed features (`crates/engine/src/liftplan.rs`, `lifts.md` §2.2).

**Geometry rules**

- **X/Z faces.** `rect.v0` is the **floor height**. Walkable floor on both sides meets the face flush at exactly `v0` across `[u0, u1)`, and the opening is clear of collidable solid up to `v1`.
- **Y faces.** `rect` is the passage footprint. The lift car or stair run passes through it.
- **Minimum sizes,** derived from the capsule in `traversal.md`:
  - Clearance `v1 − v0` is at least 2.4 m (1.8 m capsule plus 0.6 m headroom).
  - `Walkway` is at least 2 m wide.
  - `BridgeEnd` is 8–32 m wide.
  - `DeckContinuation` spans the shared deck along the whole face.
  - `LiftStop` and `StairLanding` are at least 8 × 8 m.
- **Margins.** Ports a level draws itself keep **64 m** from the face edges at both L1 and L2, so they never touch an edge or corner shared with a third cell. At L1 that is `l1::SEPARATION`: `DeckContinuation` and `BridgeEnd` spans lie in `[64 m, 4032 m)`, and a `LiftStop` is the core footprint, which keeps the same 64 m. At L2 it is `l2_port::MARGIN`, which applies to own (`Walkway`) ports only. Cascaded L2 ports are L1 ports or L1 pieces clipped to the L2 tile, so they can touch the tile's edge (§4.2).

### 2.1 Internal ports: entrances (P4.1.6)

A doorway in a mass envelope lies inside its L2 cell, not on a face, and the `Port` fields above cannot place it. Hence one appended kind (`PortKind` is append-only):

```rust
pub enum PortKind {
    // … the five kinds above …
    Entrance { plane: Q }, // doorway in an envelope face (levels.md §6.2)
}
```

- `face` is `FaceKey { level, owner: the emitting cell, axis }`. It names the level and the axis only; no cell face lies there.
- `plane` is the envelope face's coordinate along `axis` in the owner's local space. The doorway faces away from the mass, whose envelope has a face on that plane; masses are 8 m apart, so the envelope is unique.
- `rect` uses the usual `(u, v)` of the axis, in the owner's local space. `v0` is the floor outside. The opening is 4 m wide and 3 m tall, and the floor inside is 4 Q higher, which leaves 2.75 m clear.
- At L3, the block that holds the doorway lists it with its own `owner` and a block-local `plane` (`levels.md` §6.6).
- Entrances never cross a face, so Lemma B does not apply. They join N through their apron or skywalk (`levels.md` §6.2).

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

**Details as implemented (`worldgen::l1_port`, P0.7.4)**
- Every L1 X/Z opening is 4 m tall: `v1 = v0 + 4 m`. That clears the 2.4 m minimum, and because bridge floors keep 8 m from every slab, no opening meets a slab.
- **DeckContinuation** `u` spans `[64 m, 4032 m)`, the whole face within the margins.
- **BridgeEnd**, per bridge, draws in this order:
  1. the width, 4 m × `range(2, 8)`;
  2. `u0` on a 4 m step, so the port fits inside the margins;
  3. 8 floor-height tries on an **8 m** step in `[8 m, 4092 m]` (found by the validator in P0.9.2: floors 4 m apart on different faces stack a bridge slab directly on another bridge's floor, leaving it no headroom; on an 8 m grid the lower keeps 4 m). All 8 are always drawn, and the first clear one wins.
- **Fallback height:** the midpoint, on the 4 m grid, of the largest vertical gap between the slabs of **both** sides and the cell's floor and ceiling. Decks are 600 m apart only within one side.
- **Overlap:** a bridge port whose rect, extended 4 m down for the bridge slab, overlaps an earlier port is dropped. Its draws still count. Ids number only the ports that are kept.
- **Y faces:** LiftStop `rect` is the core footprint, with `(u, v) = (x, z)`.

The result is that **every face between two non-cavity L1 cells carries at least one port.** At L1, the backbone lattice is the whole grid (§6).

### 4.2 L2 faces

| Source | Ports |
|---|---|
| **Cascade** (§5) | Every L1 walkable structure that crosses the L2 face gives a port of the matching kind: deck surface, bridge deck, lift core |
| **Own ports** `[PORT_FACE_*]` | 0–2 **Walkway** (skywalk) ports per X/Z face, with `chance(lerp(0.2, 0.6, d))` each. Allowed only where both sides have the **same L1 deck** under the face. The floor is that deck's top plus 4 m × `range(0, 8)`, one of the first 8 floors |

**As implemented** (`worldgen::l2_port`, P0.8.3):
- **Cascade on an L1 face:** every L1 port, clipped to the L2 tile, with the same kind. **Inside an L1 cell:** deck and bridge pieces that reach the plane from both sides, merged along the face, give `DeckContinuation` and `BridgeEnd` ports 4 m tall at the piece's top; the lift core gives a `LiftStop` on Y faces. Cascade ports are clipped to the tile and so can touch its edge: the 64 m margin applies to own ports only.
- **Own ports:** two slots per X/Z face, each drawing (in order) presence, floor index `range(0, 8)`, width `2 m × range(2, 5)`, `u0` on a 2 m step inside a **64 m** margin, and the deck pick. The decks are those equal on both sides whose top and a floor both lie in the tile (the stair stands on the deck inside the cell). The 64 m margin equals the corridor depth below, so corridors on perpendicular faces never meet. A slot is dropped if the floor leaves the tile, the `u` range overlaps an earlier skywalk or the rect overlaps another port, or the **corridor** (below) does not fit on either side. Ports are 4 m tall.
- **Corridor:** each side reserves a 64 m deep volume from the face, the port's width wide, from the deck top to 4 m above the floor (`l2_walk::Skywalk::corridor`). It fits if the footprint is fully on deck at the deck top and no L1 `KeepOut`, collidable solid or trunk meets it (`l2_mass::Parent::fits`). A fitting corridor guarantees the realization of P0.8.4 cannot fail.
- Cascade ports whose midpoint lies inside a trunk are dropped: a trunk landing ends on the trunk's axis, and the buried part is not floor.
- Ports are numbered cascade first (sorted by floor, `u`, kind), then own.

L2 has no own ports on Y faces. Vertical movement inside an L1 cell uses L2 towers, which are internal, and the L1 core.

### 4.3 L3 and finer

L2 ports and envelope entrances become L3 boundary conditions (P4.1.1). The same rules apply. L3 cascades L2 ports that lie on its block faces, lists entrances, and derives its own `Walkway` and `StairLanding` ports from the L3 face key and the parent's mass plans (`levels.md` §6.6). Own L3 ports keep 3 m from column edges, not 8 m.

## 5. The cascade (L1 → L2 → L3)

- L1 walkable structure (deck tops, bridge decks, the lift core) is **structural**. Finer levels never remove it, and they keep its corridors open with `KeepOut` constraints over bridge decks, core landings and the region 2.4 m above deck walkways.
- Where such a structure crosses an L2 face, both L2 cells see a **forced port** at the intersection. Both compute it from the same L1 keyed features. The same mechanism carries L2 ports into L3.
- **Finer levels add collidable solid only** inside their parent's envelopes, on their own walkways and as railings along edges. Anything else they add on L1 walkable surfaces is `NO_COLLIDE`. This is what keeps the L1 network unobstructed.

## 6. Connectivity argument (versions 1–6; see §6W for version 7)

**Definitions**
- A *cavity cell* is an L1 cell inside a giant cavity.
- The **network N** is the union of L1 walkable structure (deck tops, bridge decks, lift-core stops) over all non-cavity L1 cells.

**Lemma A (L1 local connectivity).** In every non-cavity L1 cell, all L1 walkable structure and all of the cell's L1 ports lie in a single walkable component, using L1 geometry only. This holds by construction (`levels.md` §4):

1. Every deck in the cell is connected. Holes (shafts, chasms, the core) and trunk footprints each keep at least 64 m from each other and from the faces, so a deck minus its holes is a connected region. In rim cells, the cavity bite is shrunk until a flood fill confirms that the deck still touches every port-carrying face and every core stop (`levels.md` §4.6).
2. The lift core has a stop on every deck top in the cell. That connects all decks.
3. Every bridge runs from its BridgeEnd port to a core stop at its own height. If no axis-aligned route clears the trunk, the trunk segment is omitted in this cell. That fallback is deterministic, counted in stats, and expected to be rare.
4. Y-face LiftStop ports are the core itself.
5. DeckContinuation ports lie on decks, which are connected by (1).

By §5, finer levels do not obstruct any of this.

**Lemma B (agreement).** Both cells compute each port identically (§3). On both sides the walkable floor meets the port rect flush at `v0` (§2), so a port joins the two cells' components.

**Lemma C (grid connectivity).**
- Every face between two non-cavity L1 cells carries at least one port (§4.1).
- Cavity cells lie in the inner 6 × 6 × 6 L1 cells of their L0 cell, so the outer layer (the *shell*) of every L0 cell is free of cavity cells (`levels.md` §3).
- Each shell is connected (a hollow cube of thickness one cell), and adjacent L0 cells' shells share faces. The union of all shells is therefore connected.
- Pocket filling guarantees that every non-cavity cell inside an L0 cell reaches its shell through non-cavity cells.
- So the graph of non-cavity L1 cells joined by ported faces is connected.

**Theorem (global connectivity).** N is connected.
*Proof:* by A, each non-cavity L1 cell's part of N, together with its ports, is one component. By B, adjacent cells' components merge through every shared port. By C, these merges connect all non-cavity cells. ∎

**Lemma D (finer levels attach to N).** In every L2 cell, each port and each walkable surface the cell generates (masses' entrances, walkways, stairs, towers) connects **within the cell** to N. This is guaranteed by `levels.md` §5:
- masses sit only on deck tops,
- own skywalk ports exist only above a deck on both sides,
- the walkway solver links everything to the deck or core it stands on,
- the validator (P0.9) rejects any cell that fails, as a generator bug.

L3 has the same obligation toward L2 surfaces, stated as Theorem L3 in `levels.md` §6.8. For L3, "within the cell" means within the parent L2 cell, because a block buried inside a mass contains no part of N. Deliberately sealed rooms and dead ends are marked as such and are exempt.

**Corollary (horizon promise, reachability).** Every walkable surface that is not marked sealed is reachable from every other one. Every far-visible structure (deck, bridge, trunk) can be reached by walking:
- decks and bridges are part of N,
- a trunk segment passes through a deck in its cell, or, in cells with no deck, the bridge router gives one bridge a landing against the trunk (`levels.md` §4.5).

**What this does not claim.** The argument does not cover the time it takes. A 20 km walk is reachable but slow, which is why transport (lifts, trams) exists. It assumes Lemma A's construction is implemented correctly; P0.9.4 and the phase-exit check over 10⁶ cells (P0.11.5) test exactly that. Gravity regions (P5.2.4) would need this argument revisited.

## 6W. Connectivity argument for the Web (version 7)

*PW.0.3. This replaces §6 from `GEN_VERSION` 7. §6 stays as the description of versions 1–6. The lift-core guarantee (a lift core in every L1 column, decks joined by it) is gone. Its place is taken by the **skeleton**: ports on every face and arteries inside every cell (`levels.md` §4W.5).*

### 6W.1 Definitions

- An L1 cell is **live** unless it is a cavity cell (`levels.md` §3). Betweens and voids are live.
- An **L2 cell of a live L1 cell** is a cell of its 8×8×8 grid.
- The **skeleton** `S` of a live L1 cell is its tree `T` of L2 cells (`levels.md` §4W.5.2): the knot, the cells of every port artery, loop and tie.
- The **network N** is the union of the skeletons of all live cells, joined through the ports of the faces between them.

### 6W.2 Port rule

Every face between two live L1 cells carries **at least 2 ports** (`levels.md` §4W.5.1), each a distinct L2 cell of the face. They are derived from the face key (§3), and the face context is the keyed features of both cells. A face next to a cavity cell carries none.

The old minimum sizes (§2) still apply to the opening of each port: clearance of at least 2.4 m, and for an artery that carries walkers a width of at least 2 m. The opening sits on the artery's floor in the L2 cells at both sides of the face, which both cells derive from the artery type.

### 6W.3 Lemmas

**Lemma A (skeleton connectivity inside a live L1 cell).** Every port cell, the knot, and every tie start cell of a live L1 cell is in `T`, and `T` is a connected set of L2 cells (6-connectivity of the supporting axis paths).

*Proof.* `T` starts as `{K}`. A walk ends only when it enters a cell of `T`. Walks that are not finished after their step budget are completed by a BFS over the 6-neighbour grid. That grid is the whole 8×8×8 block: no cell is impassable, because voids, beams, tissue and fill only limit what is *built* and not what a *spanning artery* may cross (§4W.5.2, §4W.5.3). The grid is connected, so the BFS finds a path to `T` from every cell. Every walk therefore adds a path that is connected to `T`, and by induction `T` stays connected. A diagonal step is recorded with its supporting axis path, so the 6-connected path exists even where the geometry is oblique. A forced artery (a Road or Spine) is a straight axis path that is joined to `T` by such a walk before it is added. Hence `K` ∈ `T`, every port and tie start is in `T`, and `T` is connected. ∎

**Lemma B (agreement).** Both cells compute each port identically (§3). On both sides the walkable floor meets the port rect flush at `v0` (§2), so a port joins the two cells' components.

**Lemma C (grid connectivity).**
- Every face between two non-cavity L1 cells carries at least one port (§4.1).
- Cavity cells lie in the inner 6 × 6 × 6 L1 cells of their L0 cell, so the outer layer (the *shell*) of every L0 cell is free of cavity cells (`levels.md` §3).
- Each shell is connected (a hollow cube of thickness one cell), and adjacent L0 cells' shells share faces. The union of all shells is therefore connected.
- Pocket filling guarantees that every non-cavity cell inside an L0 cell reaches its shell through non-cavity cells.
- So the graph of non-cavity L1 cells joined by ported faces is connected.

**Theorem (global connectivity).** N is connected.
*Proof:* by A, each non-cavity L1 cell's part of N, together with its ports, is one component. By B, adjacent cells' components merge through every shared port. By C, these merges connect all non-cavity cells. ∎

**Lemma D (finer levels attach to N).** In every L2 cell, each port and each walkable surface the cell generates (masses' entrances, walkways, stairs, towers) connects **within the cell** to N. This is guaranteed by `levels.md` §5:
- masses sit only on deck tops,
- own skywalk ports exist only above a deck on both sides,
- the walkway solver links everything to the deck or core it stands on,
- the validator (P0.9) rejects any cell that fails, as a generator bug.

L3 has the same obligation toward L2 surfaces, stated as Theorem L3 in `levels.md` §6.8. For L3, "within the cell" means within the parent L2 cell, because a block buried inside a mass contains no part of N. Deliberately sealed rooms and dead ends are marked as such and are exempt.

**Corollary (horizon promise, reachability).** Every walkable surface that is not marked sealed is reachable from every other one. Every far-visible structure (deck, bridge, trunk) can be reached by walking:
- decks and bridges are part of N,
- a trunk segment passes through a deck in its cell, or, in cells with no deck, the bridge router gives one bridge a landing against the trunk (`levels.md` §4.5).

**What this does not claim.** The argument does not cover the time it takes. A 20 km walk is reachable but slow, which is why transport (lifts, trams) exists. It assumes Lemma A's construction is implemented correctly; P0.9.4 and the phase-exit check over 10⁶ cells (P0.11.5) test exactly that. Gravity regions (P5.2.4) would need this argument revisited.

## 6W. Connectivity argument for the Web (version 7)

*PW.0.3. This replaces §6 from `GEN_VERSION` 7. §6 stays as the description of versions 1–6. The lift-core guarantee (a lift core in every L1 column, decks joined by it) is gone. Its place is taken by the **skeleton**: ports on every face and arteries inside every cell (`levels.md` §4W.5).*

### 6W.1 Definitions

- An L1 cell is **live** unless it is a cavity cell (`levels.md` §3). Betweens and voids are live.
- An **L2 cell of a live L1 cell** is a cell of its 8×8×8 grid.
- The **skeleton** `S` of a live L1 cell is its tree `T` of L2 cells (`levels.md` §4W.5.2): the knot, the cells of every port artery, loop and tie.
- The **network N** is the union of the skeletons of all live cells, joined through the ports of the faces between them.

### 6W.2 Port rule

Every face between two live L1 cells carries **at least 2 ports** (`levels.md` §4W.5.1), each a distinct L2 cell of the face. They are derived from the face key (§3), and the face context is the keyed features of both cells. A face next to a cavity cell carries none.

The old minimum sizes (§2) still apply to the opening of each port: clearance of at least 2.4 m, and for an artery that carries walkers a width of at least 2 m. The opening sits on the artery's floor in the L2 cells at both sides of the face, which both cells derive from the artery type.

### 6W.3 Lemmas

**Lemma A (skeleton connectivity inside a live L1 cell).** Every port cell, the knot, and every tie start cell of a live L1 cell is in `T`, and `T` is a connected set of L2 cells (6-connectivity of the supporting axis paths).

*Proof.* `T` starts as `{K}` (or with the forced arteries of §4W.5.5, which are straight axis paths, and `K` is added to them by the first port walk, see below). A walk ends only when it enters a cell of `T`. Walks that are not finished after their step budget are completed by a BFS over the 6-neighbour grid. That grid is the whole 8×8×8 block: no cell is impassable, because voids, beams, tissue and fill only limit what is *built* and not what a *spanning artery* may cross (§4W.5.2, §4W.5.3). The grid is connected, so the BFS finds a path to `T` from every cell. Every walk therefore adds a path that is connected to `T`, and by induction `T` stays connected. A diagonal step is recorded with its supporting axis path, so the 6-connected path exists even where the geometry is oblique. Forced arteries of Roads and Spines are straight axis paths; if `T` does not yet contain `K` when the first port walk begins, the target is the nearest cell of `T` (a forced path), and `K` is joined by a final walk from `K` to `T` that is run before step 1 (it is the first draw of the artery RNG). Hence `K` ∈ `T` and `T` is connected. ∎

**Lemma B (agreement).** Both cells next to a face compute the same ports and the same artery types, so they are joined.

*Proof.* Ports, their cells and their types are a function of the face key and the keyed features of the two cells only: line features (beams are lattice lines, voids are keyed), the stack of the L0 cell (or cells: the owner is the lower cell, whose regime is used), and the Roads and Spines of the L0 lines (§3W.2). None reads either cell's generated output. The L2 cell on each side of the face is the same `(i, j)` position, and the artery's floor height at the face is a function of the type and the stack. So both L1 cells emit `ForcedPort`s for the same L2 faces, and each L2 cell reads one `ForcedPort` for each. ∎

**Lemma C (grid connectivity).**
- Every face between two live L1 cells carries ≥ 2 ports (§6W.2).
- Cavity cells lie in the inner 6×6×6 cells of their L0 cell, so the outer layer of every L0 cell (the *shell*) holds no cavity cell. No void touches an L0 face, and a Wonder or any other feature that excludes whole L1 cells must satisfy the same rule and the pocket-filling rule (`levels.md` §3).
- Each shell is connected (a hollow cube of thickness one cell), adjacent L0 cells' shells share faces, and so the union of all shells is connected.
- Pocket filling puts every L1 cell that is not a cavity in the same component as the shell.
- So the graph of live L1 cells, joined by faces that carry ports, is connected. ∎

**Theorem W (global connectivity).** The network N is connected.

*Proof.* By A, the skeleton of each live cell is connected and contains all its ports. By B, adjacent cells' skeletons are joined through each port of the shared face (two ports are two L2 cells of the face, and both belong to the skeletons on either side). By C, the live cells form one connected graph. So N is connected. ∎

Nothing in the proof depends on the stack geometry (Flat, Tilted, Vertical, Curved), on the void shapes, on the orientation policy, or on the neighbouring regime. This is why the skeleton, and not the crusts, carries the guarantee.

**Lemma D (finer levels attach to N).** In every L2 cell the cell's ports, entrances, routes and structures connect, inside the cell, to the artery cells of its parent skeleton, or to an artery port of its own. This is the order of the tissue (`levels.md` §5W, PW.0.4): reserve → route → grow → dress. The route stage connects every port of the cell (cascaded from arteries, forced and own) with walks on the slot lattice, which is almost empty at that time, so a route always exists. The validator (`worldgen::validate`) checks each cell, and a failure is a generator bug (CLAUDE.md §3.5).

For L3 the same statement holds within the parent L2 cell (`levels.md` §6.8, PW.0.5). Deliberately sealed rooms and dead ends are marked as such and are exempt. A dead end must hold a payoff (`PRN-05`).

### 6W.4 Corollaries

**Horizon promise (reachability).** Every far-visible walkable feature of a live L1 cell is tied to the cell's skeleton (§4W.5.4): every walkable beam, every crust skin piece, and every tether, drip, spire and island that touches the cell. N is connected, so each of them can be reached by walking. A crust surface is also reached through its skin ties. A knot is in `T` by construction.

**Between crossings.** Because Betweens are live and every face carries ports, there is always a route across a Between. It may be a thin catwalk or a cable rather than a stair. The engine does not need to know: those are artery types.

### 6W.5 What this does not claim

- Time: a 20 km catwalk is reachable but slow. Lifts, trams and gravity are the shortcuts (`lifts.md`, `trams.md`, `gravity.md`).
- It assumes Lemma A is implemented as written. PW.2.8 tests it over 10⁴ L1 cells with a connectivity check, and the phase-exit check (PW.12) runs the bots over many kilometres.
- Cavity cells and L0 features that exclude cells must satisfy the shell rule. A new such feature has to re-prove Lemma C.
- Gravity wells change what "down" is and so what is walkable. `gravity.md` states the extra conditions.

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
