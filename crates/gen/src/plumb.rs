//! Rails stand plumb (PW.4.10, spec `levels.md` §5W.31). A rail is a box in the frame of the
//! walkway it guards; in a tilted frame its posts would lean with the walkway, so a rail is
//! rebuilt as boxes in the upright frame of the same heading: the same footprint, a height
//! measured from the walking surface, and a bar that follows a slope as short level steps.

#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use glam::{DVec2, DVec3};

fn horizontal(v: DVec3) -> f64 {
    DVec2::new(v.x, v.z).length()
}

use crate::orient::Frame;

/// A frame whose surface normal is within this of +Y counts as walkable (the 40 degree slope
/// limit of `traversal.md`): `cos 40 = 0.766`.
pub const WALKABLE_COS: f64 = 0.766;

/// Whether the top face of a piece in `frame` is a walking surface.
pub fn walkable(frame: Frame) -> bool {
    frame.axes()[1].y >= WALKABLE_COS
}

/// Whether the frame's up is within half a degree of +Y.
pub fn is_upright(frame: Frame) -> bool {
    frame.axes()[1].y >= 0.99996
}

/// The upright frame of the same heading, as the frame itself, its horizontal axes `x'` and `z'`
/// (so that `Y` is up and `x' × Y = z'`). `None` when the frame is sideways: local x vertical
/// (`Dir` along Y) or a surface past the slope limit.
pub fn upright(frame: Frame) -> Option<(Frame, DVec3, DVec3)> {
    if !walkable(frame) {
        return None;
    }
    let up = match frame {
        Frame::Id => Frame::Id,
        Frame::Table { yaw, .. } => Frame::table(yaw, 0),
        Frame::Dir { d } => {
            if d[0] == 0 && d[2] == 0 {
                return None;
            }
            Frame::dir([d[0], 0, d[2]])
        }
    };
    let [x, _, z] = up.axes();
    Some((up, x, z))
}

/// A plumb box: `lo` and `hi` in the upright frame's coordinates (`x'`, `y`, `z'` of its
/// origin-less axes), and the world point (cell-local metres) of its `lo` corner.
#[derive(Debug, Clone, Copy)]
pub struct Piece {
    pub lo: DVec3,
    pub hi: DVec3,
    pub corner: DVec3,
}

/// The pieces that stand where the box `[lo, hi]` of `frame` (local metres, `origin` the frame's
/// origin in cell-local metres) stands, in the upright frame: `(lo, hi)` in that frame's
/// coordinates from the cell's origin (`x'`, `y`, `z'`, metres). A bar along local x is cut
/// into steps of at most `step` metres so that it follows the slope; the height of a step spans
/// the surface under it, plus the box's own height. Returns nothing for a frame that is not
/// walkable.
pub fn plumb_box(frame: Frame, origin: DVec3, lo: DVec3, hi: DVec3, step: f64) -> Vec<Piece> {
    let Some((_, hx, hz)) = upright(frame) else {
        return Vec::new();
    };
    let [ex, ey, ez] = frame.axes();
    let world = |p: DVec3| origin + ex * p.x + ey * p.y + ez * p.z;
    let d = hi - lo;
    // Cut along the longer horizontal side if it climbs (local x is the climbing axis of a
    // tilted frame; local z is level).
    let along_x = d.x >= d.z && ex.y.abs() > 1e-6;
    let n = if along_x {
        ((d.x * horizontal(ex) / step).ceil() as usize).max(1)
    } else {
        1
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (mut a, mut b) = (lo, hi);
        if along_x {
            a.x = lo.x + d.x * i as f64 / n as f64;
            b.x = lo.x + d.x * (i + 1) as f64 / n as f64;
        }
        // The four base corners decide the footprint and where the surface is.
        let base = [
            DVec3::new(a.x, lo.y, a.z),
            DVec3::new(b.x, lo.y, a.z),
            DVec3::new(a.x, lo.y, b.z),
            DVec3::new(b.x, lo.y, b.z),
        ]
        .map(world);
        let (mut mn, mut mx) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
        for p in base {
            let c = DVec3::new(p.dot(hx), p.y, p.dot(hz));
            mn = mn.min(c);
            mx = mx.max(c);
        }
        mx.y += d.y;
        out.push(Piece {
            lo: mn,
            hi: mx,
            corner: hx * mn.x + DVec3::Y * mn.y + hz * mn.z,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_post_in_a_tilted_frame_stands_plumb() {
        let frame = Frame::table(8, 4);
        // A 0.1 m post 1.125 m high, standing on the frame's surface at (10, 0, 10).
        let boxes = plumb_box(
            frame,
            DVec3::new(100.0, 50.0, 100.0),
            DVec3::new(10.0, 0.0, 10.0),
            DVec3::new(10.1, 1.125, 10.1),
            1.0,
        );
        assert_eq!(boxes.len(), 1);
        let (lo, hi) = (boxes[0].lo, boxes[0].hi);
        assert!((hi.y - lo.y - 1.125).abs() < 0.05, "height {}", hi.y - lo.y);
        assert!(hi.x - lo.x < 0.15 && hi.z - lo.z < 0.15, "{lo:?} {hi:?}");
    }

    #[test]
    fn a_bar_up_a_slope_is_cut_into_steps_that_follow_it() {
        let frame = Frame::table(0, 6);
        let slope = frame.axes()[0].y / horizontal(frame.axes()[0]);
        let boxes = plumb_box(
            frame,
            DVec3::ZERO,
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(8.0, 0.07, 0.1),
            0.5,
        );
        assert!(boxes.len() >= 14, "{}", boxes.len());
        // Each step is level and as low as the surface under its low end.
        let mut last = f64::MIN;
        for b in &boxes {
            assert!(b.lo.y >= last - 1e-9, "steps climb");
            assert!(b.hi.y - b.lo.y < 0.07 + slope * 0.5 + 1e-6);
            last = b.lo.y;
        }
    }

    #[test]
    fn a_sideways_frame_has_no_walking_surface() {
        assert!(
            plumb_box(
                Frame::dir([0, 1, 0]),
                DVec3::ZERO,
                DVec3::ZERO,
                DVec3::ONE,
                1.0
            )
            .is_empty()
        );
        assert!(
            !walkable(Frame::dir([1, 1, 0])),
            "45 degrees is past the limit"
        );
        assert!(walkable(Frame::dir([3, 1, 0])), "18 degrees is walkable");
        assert!(is_upright(Frame::Id) && !is_upright(Frame::table(0, 1)));
    }
}
