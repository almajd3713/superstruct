//! Openings where walkable space crosses a cell face. Spec: `docs/spec/ports.md`.

use crate::cell::FaceKey;
use crate::l1_artery::ArteryType;
use crate::prim::{Q, QRect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Port {
    pub face: FaceKey,
    /// Index on this face, in deterministic order.
    pub id: u16,
    pub kind: PortKind,
    /// Opening in the face plane `(u, v)`. On X/Z faces `v0` is the floor height.
    pub rect: QRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PortKind {
    Walkway,
    DeckContinuation,
    BridgeEnd,
    StairLanding,
    LiftStop,
    /// A doorway in a mass envelope, `plane` being the envelope face it pierces along the
    /// face's axis. It lies inside its cell, not on a cell face (`levels.md` §6.2).
    Entrance {
        plane: Q,
    },
    /// An artery of the Web skeleton crossing the face (`levels.md` §4W.5). The rect is the
    /// whole face of the L2 cell or the L2 cell's side of the L1 face; the floor and width
    /// are decided by the L2 tissue.
    Artery {
        kind: ArteryType,
    },
}

impl PortKind {
    /// Declaration order, for sorting ports of different kinds.
    pub const fn rank(self) -> u8 {
        match self {
            Self::Walkway => 0,
            Self::DeckContinuation => 1,
            Self::BridgeEnd => 2,
            Self::StairLanding => 3,
            Self::LiftStop => 4,
            Self::Entrance { .. } => 5,
            Self::Artery { .. } => 6,
        }
    }
}
