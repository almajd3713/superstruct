//! Model synthesis over a lattice of slots (`docs/spec/levels.md` §6.7).
//!
//! The solver knows nothing about L3 content. A tile set gives each tile a socket on each of
//! six sides and a weight; the only adjacency rule is that sockets on a shared side are equal.
//! Callers pass the initial domain of every slot (boundary conditions are domains restricted to
//! the tiles that fit them), and get back one tile per slot or a contradiction. There is no
//! backtracking: a retry is a new attempt with a new RNG, chosen by the caller.
//!
//! Sides are numbered -z, +x, +z, -x, down, up, and a slot's index is `(t · nz + z) · nx + x`.
//! Everything is deterministic: propagation order, the selection rule (fewest remaining tiles,
//! lowest index first) and one weighted draw per collapse.

use crate::rng::Rng;

/// Tiles are limited to four 64-bit words.
pub const WORDS: usize = 4;
pub const MAX_TILES: usize = WORDS * 64;
pub const SIDES: usize = 6;
/// Sockets are small integers below this.
pub const MAX_SOCKETS: usize = 16;

/// A set of tiles, one bit each.
pub type Domain = [u64; WORDS];

pub const EMPTY: Domain = [0; WORDS];

pub fn domain_of(tiles: impl IntoIterator<Item = usize>) -> Domain {
    let mut d = EMPTY;
    for t in tiles {
        d[t / 64] |= 1 << (t % 64);
    }
    d
}

pub fn count(d: &Domain) -> u32 {
    d.iter().map(|w| w.count_ones()).sum()
}

pub fn is_empty(d: &Domain) -> bool {
    d.iter().all(|&w| w == 0)
}

pub fn contains(d: &Domain, tile: usize) -> bool {
    d[tile / 64] >> (tile % 64) & 1 == 1
}

fn and(a: &Domain, b: &Domain) -> Domain {
    std::array::from_fn(|i| a[i] & b[i])
}

fn or_assign(a: &mut Domain, b: &Domain) {
    for i in 0..WORDS {
        a[i] |= b[i];
    }
}

fn intersects(a: &Domain, b: &Domain) -> bool {
    a.iter().zip(b).any(|(x, y)| x & y != 0)
}

/// The side opposite `side`.
pub const fn opposite(side: usize) -> usize {
    match side {
        0 => 2,
        1 => 3,
        2 => 0,
        3 => 1,
        4 => 5,
        _ => 4,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dims {
    pub nx: usize,
    pub nt: usize,
    pub nz: usize,
}

impl Dims {
    pub const fn len(&self) -> usize {
        self.nx * self.nt * self.nz
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn index(&self, x: usize, t: usize, z: usize) -> usize {
        (t * self.nz + z) * self.nx + x
    }

    pub const fn coords(&self, i: usize) -> (usize, usize, usize) {
        (i % self.nx, i / (self.nx * self.nz), i / self.nx % self.nz)
    }

    /// The slot beside `i` on `side`, if it is inside the lattice.
    pub fn neighbour(&self, i: usize, side: usize) -> Option<usize> {
        let (x, t, z) = self.coords(i);
        match side {
            0 => z.checked_sub(1).map(|z| self.index(x, t, z)),
            1 => (x + 1 < self.nx).then(|| self.index(x + 1, t, z)),
            2 => (z + 1 < self.nz).then(|| self.index(x, t, z + 1)),
            3 => x.checked_sub(1).map(|x| self.index(x, t, z)),
            4 => t.checked_sub(1).map(|t| self.index(x, t, z)),
            _ => (t + 1 < self.nt).then(|| self.index(x, t + 1, z)),
        }
    }
}

/// Tiles with their sockets and weights, and the precomputed masks.
#[derive(Debug, Clone)]
pub struct TileSet {
    sockets: Vec<[u8; SIDES]>,
    weights: Vec<u32>,
    /// `masks[side][socket]`: the tiles with that socket on that side.
    masks: [[Domain; MAX_SOCKETS]; SIDES],
    /// `used[side]`: the sockets that at least one tile has on that side.
    used: [Vec<u8>; SIDES],
}

impl TileSet {
    /// `tiles` is `(sockets per side, weight)`. Panics (a tile-set bug) above [`MAX_TILES`]
    /// tiles or [`MAX_SOCKETS`] sockets.
    pub fn new(tiles: &[([u8; SIDES], u32)]) -> Self {
        assert!(tiles.len() <= MAX_TILES, "too many tiles");
        let mut masks = [[EMPTY; MAX_SOCKETS]; SIDES];
        for (t, (s, _)) in tiles.iter().enumerate() {
            for side in 0..SIDES {
                assert!(usize::from(s[side]) < MAX_SOCKETS, "socket out of range");
                masks[side][usize::from(s[side])][t / 64] |= 1 << (t % 64);
            }
        }
        let used = std::array::from_fn(|side| {
            (0..u8::try_from(MAX_SOCKETS).unwrap_or(u8::MAX))
                .filter(|&s| !is_empty(&masks[side][usize::from(s)]))
                .collect()
        });
        Self {
            used,
            sockets: tiles.iter().map(|t| t.0).collect(),
            weights: tiles.iter().map(|t| t.1).collect(),
            masks,
        }
    }

    pub fn len(&self) -> usize {
        self.sockets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sockets.is_empty()
    }

    pub fn socket(&self, tile: usize, side: usize) -> u8 {
        self.sockets[tile][side]
    }

    pub fn weight(&self, tile: usize) -> u32 {
        self.weights[tile]
    }

    /// Tiles whose socket on `side` is `socket`.
    pub fn with_socket(&self, side: usize, socket: u8) -> Domain {
        self.masks[side][usize::from(socket)]
    }

    /// The tiles a neighbour on `side` of a slot holding `d` may hold: those whose socket on the
    /// opposite side equals some socket that `d` still has on `side`.
    fn allowed(&self, d: &Domain, side: usize) -> Domain {
        let mut out = EMPTY;
        for &s in &self.used[side] {
            let s = usize::from(s);
            if intersects(d, &self.masks[side][s]) {
                or_assign(&mut out, &self.masks[opposite(side)][s]);
            }
        }
        out
    }
}

/// An attempt failed: a slot ran out of tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contradiction {
    pub slot: usize,
}

/// Buffers reused between attempts.
#[derive(Debug, Default)]
pub struct Scratch {
    queue: std::collections::VecDeque<usize>,
    queued: Vec<bool>,
    weights: Vec<u32>,
    tiles: Vec<usize>,
    /// Tiles left per slot, kept current by the loop in [`solve_active`].
    counts: Vec<u16>,
    /// Slots that may still hold more than one tile, in index order.
    open: Vec<usize>,
}

/// Restricts `domains` until every slot agrees with its neighbours (AC-3, FIFO, seeded in slot
/// order). `Err` names the first slot that emptied.
pub fn propagate_all(
    ts: &TileSet,
    dims: Dims,
    domains: &mut [Domain],
    active: Option<&[bool]>,
    scratch: &mut Scratch,
) -> Result<(), Contradiction> {
    scratch.queue.clear();
    scratch.queued.clear();
    scratch.queued.resize(dims.len(), false);
    for i in 0..dims.len() {
        if active.is_none_or(|a| a[i]) {
            scratch.queued[i] = true;
            scratch.queue.push_back(i);
        }
    }
    run_queue(ts, dims, domains, active, scratch)
}

fn run_queue(
    ts: &TileSet,
    dims: Dims,
    domains: &mut [Domain],
    active: Option<&[bool]>,
    scratch: &mut Scratch,
) -> Result<(), Contradiction> {
    while let Some(i) = scratch.queue.pop_front() {
        scratch.queued[i] = false;
        for side in 0..SIDES {
            let Some(j) = dims
                .neighbour(i, side)
                .filter(|&j| active.is_none_or(|a| a[j]))
            else {
                continue;
            };
            let allowed = ts.allowed(&domains[i], side);
            let next = and(&domains[j], &allowed);
            if next != domains[j] {
                if is_empty(&next) {
                    return Err(Contradiction { slot: j });
                }
                if scratch.counts.len() == dims.len() {
                    scratch.counts[j] = u16::try_from(count(&next)).unwrap_or(u16::MAX);
                }
                domains[j] = next;
                if !scratch.queued[j] {
                    scratch.queued[j] = true;
                    scratch.queue.push_back(j);
                }
            }
        }
    }
    Ok(())
}

/// Collapses every slot of `domains` (initial domains in, one tile per slot out) with weighted
/// draws from `rng`. The initial propagation is part of the attempt.
pub fn solve(
    ts: &TileSet,
    dims: Dims,
    domains: &mut [Domain],
    rng: &mut Rng,
    scratch: &mut Scratch,
) -> Result<(), Contradiction> {
    solve_active(ts, dims, domains, None, rng, scratch)
}

/// [`solve`] over the slots where `active` is true. The others take no part: they neither
/// constrain their neighbours nor get collapsed (the caller has already folded their effect
/// into the domains of the active slots).
pub fn solve_active(
    ts: &TileSet,
    dims: Dims,
    domains: &mut [Domain],
    active: Option<&[bool]>,
    rng: &mut Rng,
    scratch: &mut Scratch,
) -> Result<(), Contradiction> {
    debug_assert_eq!(domains.len(), dims.len());
    let on = |i: usize| active.is_none_or(|a| a[i]);
    if let Some(i) = (0..dims.len()).find(|&i| on(i) && is_empty(&domains[i])) {
        return Err(Contradiction { slot: i });
    }
    scratch.counts.clear();
    propagate_all(ts, dims, domains, active, scratch)?;
    scratch.counts.extend(
        domains
            .iter()
            .map(|d| u16::try_from(count(d)).unwrap_or(u16::MAX)),
    );
    scratch.open.clear();
    scratch
        .open
        .extend((0..dims.len()).filter(|&i| on(i) && scratch.counts[i] > 1));
    loop {
        // Fewest remaining tiles above one, lowest index on ties.
        let counts = &scratch.counts;
        scratch.open.retain(|&i| counts[i] > 1);
        let mut pick: Option<(u16, usize)> = None;
        for &i in &scratch.open {
            let n = counts[i];
            if pick.is_none_or(|(best, _)| n < best) {
                pick = Some((n, i));
            }
        }
        let Some((_, i)) = pick else {
            return Ok(());
        };
        scratch.tiles.clear();
        scratch.weights.clear();
        for t in 0..ts.len() {
            if contains(&domains[i], t) {
                scratch.tiles.push(t);
                scratch.weights.push(ts.weights[t]);
            }
        }
        let k = rng.pick_weighted(&scratch.weights).unwrap_or(0);
        domains[i] = domain_of([scratch.tiles[k]]);
        scratch.counts[i] = 1;
        // Every flag is false again after a clean run, so the queue only needs the new slot.
        scratch.queue.push_back(i);
        scratch.queued[i] = true;
        run_queue(ts, dims, domains, active, scratch)?;
    }
}

/// The tile of a collapsed slot.
pub fn tile_of(d: &Domain) -> Option<usize> {
    (count(d) == 1).then(|| {
        d.iter()
            .enumerate()
            .find(|(_, w)| **w != 0)
            .map_or(0, |(k, w)| k * 64 + w.trailing_zeros() as usize)
    })
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    /// Every pair of neighbouring tiles in `domains` has equal sockets on the shared side.
    fn assert_consistent(ts: &TileSet, dims: Dims, domains: &[Domain]) {
        for i in 0..dims.len() {
            let a = tile_of(&domains[i]).expect("collapsed");
            for side in 0..SIDES {
                if let Some(j) = dims.neighbour(i, side) {
                    let b = tile_of(&domains[j]).expect("collapsed");
                    assert_eq!(
                        ts.socket(a, side),
                        ts.socket(b, opposite(side)),
                        "slots {i} and {j} on side {side}"
                    );
                }
            }
        }
    }

    /// Wall, open and door on the four horizontal sides: 81 tiles, floor above and below.
    fn rooms() -> TileSet {
        let mut tiles = Vec::new();
        for code in 0..81u32 {
            let s = |k: u32| u8::try_from(code / 3u32.pow(k) % 3).unwrap();
            tiles.push(([s(0), s(1), s(2), s(3), 0, 0], 1 + code % 5));
        }
        TileSet::new(&tiles)
    }

    fn all(ts: &TileSet, n: usize) -> Vec<Domain> {
        vec![domain_of(0..ts.len()); n]
    }

    const DIMS: Dims = Dims {
        nx: 4,
        nt: 3,
        nz: 4,
    };

    #[test]
    fn indexing_round_trips_and_neighbours_are_symmetric() {
        for i in 0..DIMS.len() {
            let (x, t, z) = DIMS.coords(i);
            assert_eq!(DIMS.index(x, t, z), i);
            for side in 0..SIDES {
                if let Some(j) = DIMS.neighbour(i, side) {
                    assert_eq!(DIMS.neighbour(j, opposite(side)), Some(i));
                }
            }
        }
        assert_eq!(DIMS.neighbour(0, 3), None);
        assert_eq!(DIMS.neighbour(0, 0), None);
        assert_eq!(DIMS.neighbour(0, 4), None);
    }

    #[test]
    fn a_free_tile_set_always_solves_and_neighbours_match() {
        let ts = rooms();
        for seed in 0..40 {
            let mut d = all(&ts, DIMS.len());
            let mut rng = Rng::new(seed);
            solve(&ts, DIMS, &mut d, &mut rng, &mut Scratch::default()).unwrap();
            assert_consistent(&ts, DIMS, &d);
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_result_and_other_seeds_differ() {
        let ts = rooms();
        let run = |seed| {
            let mut d = all(&ts, DIMS.len());
            solve(
                &ts,
                DIMS,
                &mut d,
                &mut Rng::new(seed),
                &mut Scratch::default(),
            )
            .unwrap();
            d
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
    }

    #[test]
    fn fixed_boundary_sockets_are_respected() {
        let ts = rooms();
        let mut d = all(&ts, DIMS.len());
        // Every outer side is a wall (socket 0).
        for i in 0..DIMS.len() {
            for side in 0..4 {
                if DIMS.neighbour(i, side).is_none() {
                    d[i] = and(&d[i], &ts.with_socket(side, 0));
                }
            }
        }
        solve(&ts, DIMS, &mut d, &mut Rng::new(3), &mut Scratch::default()).unwrap();
        assert_consistent(&ts, DIMS, &d);
        for i in 0..DIMS.len() {
            let t = tile_of(&d[i]).unwrap();
            for side in 0..4 {
                if DIMS.neighbour(i, side).is_none() {
                    assert_eq!(ts.socket(t, side), 0);
                }
            }
        }
    }

    #[test]
    fn incompatible_fixed_tiles_are_a_contradiction() {
        // Two tile kinds that cannot touch: sockets 0 on every side, and sockets 1.
        let ts = TileSet::new(&[([0; 6], 1), ([1; 6], 1)]);
        let dims = Dims {
            nx: 3,
            nt: 1,
            nz: 1,
        };
        let mut d = vec![domain_of([0]), domain_of([0, 1]), domain_of([1])];
        let err = solve(&ts, dims, &mut d, &mut Rng::new(1), &mut Scratch::default());
        assert!(err.is_err());
        let mut d = vec![domain_of([1]), domain_of([0, 1]), domain_of([1])];
        solve(&ts, dims, &mut d, &mut Rng::new(1), &mut Scratch::default()).unwrap();
        assert!(d.iter().all(|x| tile_of(x) == Some(1)));
    }

    #[test]
    fn alternating_tiles_form_a_checkerboard_from_one_fixed_slot() {
        // Each tile has the opposite socket on opposite sides, so a neighbour on any side must be
        // the other tile and parity alternates.
        let ts = TileSet::new(&[([0, 0, 1, 1, 0, 1], 1), ([1, 1, 0, 0, 1, 0], 1)]);
        let dims = Dims {
            nx: 4,
            nt: 2,
            nz: 4,
        };
        let mut d = vec![domain_of([0, 1]); dims.len()];
        d[0] = domain_of([0]);
        solve(&ts, dims, &mut d, &mut Rng::new(9), &mut Scratch::default()).unwrap();
        assert_consistent(&ts, dims, &d);
        for i in 0..dims.len() {
            let (x, t, z) = dims.coords(i);
            assert_eq!(tile_of(&d[i]), Some((x + t + z) % 2), "slot {i}");
        }
    }

    #[test]
    fn weights_bias_the_choice() {
        let ts = TileSet::new(&[([0; 6], 99), ([0; 6], 1)]);
        let dims = Dims {
            nx: 8,
            nt: 8,
            nz: 8,
        };
        let mut heavy = 0;
        for seed in 0..10 {
            let mut d = vec![domain_of([0, 1]); dims.len()];
            solve(
                &ts,
                dims,
                &mut d,
                &mut Rng::new(seed),
                &mut Scratch::default(),
            )
            .unwrap();
            heavy += d.iter().filter(|x| tile_of(x) == Some(0)).count();
        }
        let share = heavy as f64 / (10 * dims.len()) as f64;
        assert!((0.96..=1.0).contains(&share), "{share}");
    }
}
