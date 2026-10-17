//! Openings where walkable space crosses a cell face. Spec: `docs/spec/ports.md`.

use crate::cell::FaceKey;
use crate::prim::QRect;

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
}
