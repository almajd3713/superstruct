# Ladders (PW.5, CLB-01..07, LAD-01..15)

The rules of climbing are in `traversal.md` §ladders (start within 0.5 m of the rungs facing them,
forward moves where you look, 0.8 m/s, no falls, rest every 24 m, crouch 1.2 m). This file says
how the generator and the controller make them.

## The corridor

A ladder is a `FeatureKind::LADDER` box (not drawn, not collidable): the space in front of the
rungs that a climber stands in, 0.9 m deep, 0.8 m wide and as tall as the ladder. Its material id
is the side the rungs are on in the box's frame (0 -z, 1 +x, 2 +z, 3 -x). A corridor must have an
upright frame (a yaw only); tilted lattices have none, which is also why a tilted interior is bare.

Controller (`controller.rs`): walking into a corridor with the wish direction within 60 degrees of
the rungs' side puts the player on the ladder (`try_attach`). On it: no gravity, no walking off,
the climb input (forward times the sign of the look direction's vertical part, up unless looking
more than 9 degrees down) moves the player at `LADDER_SPEED` (0.8 m/s), held against the rungs.
At the top the player steps over the edge onto the floor beyond the rungs, or back onto the floor
behind, whichever has a floor; at the bottom they step back 0.5 m. Only the ends let go.
`SolidSource::ladders` serves the corridors near the player; `CollisionChunk` collects them from
a chunk's primitives (L2 and L3).

Crouch (CLB-07): where the standing capsule (1.8 m) would be pushed by a ceiling and the 1.2 m
capsule fits, here or 0.5 m ahead, the controller crouches (eye 1.05 m) and stands when there is
room again.

## Families (`paths_v/ladders.rs`, ids 40 + LAD number)

Built as vertical path types in a column. The corridor stands at plan `u` 0.3 to 1.2 with the
rungs on +u; a platform with rails at `u` -2 to 0.3 at the foot and at the top, so the route runs
from the column centre through the corridor and back to the column centre. A climb is a chain of
ladders of at most 24 m with a rest platform between them (CLB-06); the next ladder stands 1.5 m
to the side. A ladder well, a ladder chain and a façade ladder break every 4 m, 3.5 m and 4 m.

| LAD | Built as |
|---|---|
| 01 wall ladder | rails and rungs on a concrete wall panel |
| 02 caged | LAD-01 with hoops every metre from 2.2 m up and two straps |
| 03 ladder well | a deck with the corridor left open every 4 m |
| 04 ship's ladder | **deviation:** a plain wall ladder (the 60 to 70 degree lean needs a sloped corridor) |
| 05 staples | staples alternately left and right every 0.35 m, no rails |
| 06 pipe rungs | rungs across a pipe slab |
| 07 chain ladder | two thin chains and rungs, no wall (the sway is not made) |
| 08 pillar climb | a thick pillar slab with the rungs on it; chained with rests |
| 09 beside a dead lift | a four-post lift frame with a cab stuck half way, beside the ladder |
| 10 ladder chain | runs of 3.5 m with a ledge each |
| 11 into a roof hatch | a slab over the top with the way on beside it |
| 12 between balconies | balconies every 4 m on the face beside the ladder |
| 13 helical rungs | a drum slab; the rungs are drawn slanting across it (the corridor is straight) |
| 14 gravity-gate ladder | `gates::gate_ladder` (`GEN_VERSION` 33): in a gate of kind Ladder whose membrane is horizontal (the zone's axis is y), a shaft of 18 m through the zone: a climbing corridor (`LADDER`, rungs on +x), a wall of 0.6 m behind the rungs, a rung every 5/16 m (passable) and a landing at each end. In the field of the low side the climb is up; past the membrane it is down, and the far landing, the deck's underside, is a floor from that side. Gates of other axes carry none. *Not exercised in the engine:* whether the climb carries a walker through the turn is for the 🎮 review |
| 15 ladder across a lull ceiling | `weightless` drift lanes (`GEN_VERSION` 33): a rail along the tube's ceiling (3 m under the top of the ring) and a rung of 4 m across it every 4 m, passable. *Not built:* the pull along the rungs (a handrail mechanic), so far you drift along it |

Ladder ids weigh a quarter of a stair id in the choice of a vertical type; the stratum's LADDER
weight decides whether a stratum has them. L3 interiors carry corridors in their ladder wells
(L3-20 and the roof hatches), and their rails and rungs no longer collide.
