//! Links between the walking nodes of an L2 cell (`docs/spec/levels.md` §5.3 steps 1–2 and the
//! "Links" paragraph of §6.2): a spanning tree, plus loops that each fire with probability 0.3.
//! A link is a clear 4 m wide path over deck; it only reserves space (`KeepOut`), since every
//! node already reaches the network on its own.

use crate::cell::{CellKey, Level};
use crate::hash::hash;
use crate::l2_mass::{Parent, overlaps};
use crate::prim::{Q, QAabb, QVec3};
use crate::rng::Rng;
use crate::salt;
use crate::seed::WorldSeed;

const WIDTH: Q = 64;
const HEIGHT: Q = 64;
/// Nodes sit 2 m inside a face.
pub const INSET: Q = 32;
/// Chance that a loop candidate becomes a link.
pub const LOOP_CHANCE: f32 = 0.3;

/// A point where people arrive: plan position and the deck top it stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub top: Q,
    pub x: Q,
    pub z: Q,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LinkStats {
    pub nodes: u32,
    pub tree: u32,
    pub candidates: u32,
    pub fired: u32,
    pub dropped: u32,
}

fn dist(a: &Node, b: &Node) -> i64 {
    i64::from((a.x - b.x).abs()) + i64::from((a.z - b.z).abs())
}

/// Prim's algorithm from node 0: the unconnected node nearest the tree joins next; ties go to
/// the lower new index, then the lower tree index. Edges come in the order they are added.
fn spanning_tree(nodes: &[Node]) -> Vec<(usize, usize)> {
    let mut in_tree = vec![false; nodes.len()];
    in_tree[0] = true;
    let mut edges = Vec::new();
    for _ in 1..nodes.len() {
        let mut best: Option<(i64, usize, usize)> = None;
        for (j, nj) in nodes.iter().enumerate().filter(|&(j, _)| !in_tree[j]) {
            for (i, ni) in nodes.iter().enumerate().filter(|&(i, _)| in_tree[i]) {
                let key = (dist(ni, nj), j, i);
                if best.is_none_or(|b| key < b) {
                    best = Some(key);
                }
            }
        }
        let Some((_, j, i)) = best else {
            break;
        };
        in_tree[j] = true;
        edges.push((i, j));
    }
    edges
}

/// The L-shaped path from `a` to `b`, x first or z first, as two boxes.
fn path(a: &Node, b: &Node, x_first: bool) -> [QAabb; 2] {
    let h = WIDTH / 2;
    let span = |p: Q, q: Q| (p.min(q) - h, p.max(q) + h);
    let boxed = |x: (Q, Q), z: (Q, Q)| {
        QAabb::new(
            QVec3::new(x.0, a.top, z.0),
            QVec3::new(x.1, a.top + HEIGHT, z.1),
        )
    };
    if x_first {
        [
            boxed(span(a.x, b.x), (a.z - h, a.z + h)),
            boxed((b.x - h, b.x + h), span(a.z, b.z)),
        ]
    } else {
        [
            boxed((a.x - h, a.x + h), span(a.z, b.z)),
            boxed(span(a.x, b.x), (b.z - h, b.z + h)),
        ]
    }
}

/// The first of the two L-paths that stands fully on deck, clear of the parent and of every
/// collidable `solid` of the cell (envelopes included).
fn realize(ctx: &Parent, solid: &[QAabb], cell_size: Q, a: &Node, b: &Node) -> Option<[QAabb; 2]> {
    [true, false]
        .into_iter()
        .map(|x_first| path(a, b, x_first))
        .find(|boxes| {
            boxes.iter().all(|bb| {
                bb.min.x >= 0
                    && bb.min.z >= 0
                    && bb.max.x <= cell_size
                    && bb.max.z <= cell_size
                    && ctx.fits(bb)
                    && !solid.iter().any(|s| overlaps(s, bb))
            })
        })
}

/// Links for the nodes of one cell, grouped by deck top ascending and kept in the order given
/// within a deck. Returns the clearance boxes and the statistics.
#[allow(clippy::cast_possible_truncation)] // a handful of nodes per cell
pub(crate) fn links(
    seed: WorldSeed,
    cell: CellKey,
    ctx: &Parent,
    solid: &[QAabb],
    cell_size: Q,
    nodes: &[Node],
) -> (Vec<QAabb>, LinkStats) {
    debug_assert_eq!(cell.level, Level::L2);
    let mut rng = Rng::new(hash(seed.0, Level::L2, cell.coords, salt::WALKWAY));
    let mut out = Vec::new();
    let mut stats = LinkStats::default();
    let mut tops: Vec<Q> = nodes.iter().map(|n| n.top).collect();
    tops.sort_unstable();
    tops.dedup();
    for top in tops {
        let group: Vec<Node> = nodes.iter().filter(|n| n.top == top).copied().collect();
        stats.nodes += group.len() as u32;
        if group.len() < 2 {
            continue;
        }
        let tree = spanning_tree(&group);
        let mut edges: Vec<(usize, usize)> = tree.clone();
        stats.tree += tree.len() as u32;
        let mut candidates: Vec<(usize, usize)> = Vec::new();
        let same = |a: (usize, usize), b: (usize, usize)| a == b || (a.1, a.0) == b;
        for i in 0..group.len() {
            // Its nearest node that no tree edge already joins to it.
            let nearest = (0..group.len())
                .filter(|&j| j != i && !tree.iter().any(|&e| same(e, (i, j))))
                .min_by_key(|&j| (dist(&group[i], &group[j]), j));
            let Some(j) = nearest else {
                continue;
            };
            let pair = (i, j);
            if candidates.iter().any(|&e| same(e, pair)) {
                continue;
            }
            candidates.push(pair);
        }
        stats.candidates += candidates.len() as u32;
        for pair in candidates {
            if rng.chance(LOOP_CHANCE) {
                stats.fired += 1;
                edges.push(pair);
            }
        }
        for (i, j) in edges {
            match realize(ctx, solid, cell_size, &group[i], &group[j]) {
                Some(boxes) => out.extend(boxes),
                None => stats.dropped += 1,
            }
        }
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: Q, z: Q) -> Node {
        Node { top: 0, x, z }
    }

    #[test]
    fn the_tree_spans_and_is_deterministic() {
        let nodes = [n(0, 0), n(100, 0), n(100, 300), n(-50, 10), n(400, 400)];
        let t = spanning_tree(&nodes);
        assert_eq!(t.len(), nodes.len() - 1);
        assert_eq!(t, spanning_tree(&nodes));
        let mut reached = vec![false; nodes.len()];
        reached[0] = true;
        for &(i, j) in &t {
            assert!(reached[i], "edges are added from the tree");
            reached[j] = true;
        }
        assert!(reached.iter().all(|&r| r));
    }

    #[test]
    fn ties_go_to_the_lower_index() {
        // Nodes 1 and 2 are equally far from node 0.
        let t = spanning_tree(&[n(0, 0), n(10, 0), n(-10, 0)]);
        assert_eq!(t[0], (0, 1));
    }

    #[test]
    fn an_l_path_has_two_boxes_that_meet() {
        let [a, b] = path(&n(0, 0), &n(200, 300), true);
        assert!(a.max.x > 200 && b.max.z > 300);
        assert_eq!(a.min.y, 0);
        assert_eq!(a.max.y, HEIGHT);
    }
}
