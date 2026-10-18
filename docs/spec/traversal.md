# Spec: traversal

**Status: numbers approved by the user on 2026-10-17 (P0.2.8): no jump, 15 m fall-death height, 30 m/s lifts, everything else as drafted. Tram acceleration raised from 1.5 to 6 m/s² by the user on 2026-10-18, so that the tram trip time below holds.**

How the player moves, and the dimensions the generator must respect so that every port and walkway is usable. Every number here is tunable, but player-feel numbers change only with the user's agreement (CLAUDE.md §8). Generator-facing numbers (§6) change output, so they also need a `GEN_VERSION` bump.

## 1. Player body

| Parameter | Value | Notes |
|---|---|---|
| Capsule height | 1.8 m | total, including both hemispheres |
| Capsule radius | 0.3 m | |
| Eye height | 1.65 m | above the capsule bottom |
| Gravity | 9.81 m/s² along −Y | |

## 2. Movement

| Parameter | Value | Notes |
|---|---|---|
| Walk speed | 1.4 m/s | the default, a calm pace |
| Run speed | 4.0 m/s | held key |
| Ground acceleration | reaches target speed in 0.35 s | ease-in, no snap |
| Ground deceleration | stops in 0.25 s | |
| Air control | 10% of ground acceleration | barely steerable while falling |
| Step-up | 0.45 m | vertical ledge climbed without input |
| Step-down snap | 0.45 m | stays glued to stairs and ramps going down |
| Max walkable slope | 40° | steeper slopes slide the player down |
| Jump | **none** (§2.1) | |

### 2.1 Why no jump

- **Zen means no friction** (CLAUDE.md §9). Without a jump there is no platforming and no failed jumps.
- **The connectivity spec assumes it.** Ports, step-up and stairs define what is reachable. A jump would make places reachable that the generator never connected, and would invite falls.
- **Small obstacles are handled by step-up.** Step-up (0.45 m) and stairs cover everything the generator builds.

If a jump is wanted later, a small hop (0.5 m apex) is the conservative choice. It would not change any generator dimension.

### 2.2 Climbing (PW.0.6; CLB-01..07)

*The user approved ladders at 0.8 m/s on 2026-10-18; speeds are tunable later. Crouch height 1.2 m is still provisional.*

There is no new button.

| Parameter | Value | Notes |
|---|---|---|
| Start (CLB-01) | walk into a ladder while facing it | within 0.5 m of the ladder's face, the view within 50° of the ladder's normal |
| Move (CLB-02) | forward moves you along the view's component on the ladder axis | looking up climbs, looking down descends; backward is the opposite; sideways is ignored |
| Climb speed (CLB-05) | **0.8 m/s** | for rungs and staples; ship's ladders (60–70°) 1.0 m/s along the slope; chain ladders 0.6 m/s |
| Ends (CLB-03) | reaching the top or bottom steps the player off onto the landing | a short scripted step of 0.6 m over 0.4 s, onto a landing at least 1.2 × 1.2 m |
| No falls (CLB-04) | leaving a ladder mid-way is impossible | the only exits are the two ends. Movement keys that do not move along the ladder do nothing |
| Rests (CLB-06) | a rest cage or platform every ≤ 30 m | the generator places one when a ladder is longer than 30 m (`levels.md` §4W.5.3, ladder chain, and L4) |
| Ladder dimensions | rung spacing 0.30 m; clear width 0.5–0.8 m; cage inner radius 0.45 m; clearance in front of the rungs ≥ 0.7 m | generator-facing (§6) |
| Capsule on a ladder | held 0.4 m from the rungs, upright along the local up | collision against the walls is off while on the ladder; the cage is a visual |

A ladder is a line of climb points with an axis and two end landings (a **ladder segment**). The ladder families (LAD-01..15) are built from segments; nothing about the mechanic depends on the family. A ladder in a gravity gate (LAD-14) or in a lull tube (LAD-15) follows the local `up` (`gravity.md`).

**Auto-crouch (CLB-07).** Where the clear height is below 1.9 m the capsule shrinks to **1.2 m** by itself, with eye height 1.05 m and a walk speed of 0.9 m/s *(provisional)*. It stands up again when the clear height allows 1.8 m. Crawl ducts, vents and gaps under machines have a clear height of **at least 1.3 m** (the generator minimum, §6); between 1.3 and 1.9 m the player crouches. Below 1.3 m a passage is not walkable and the generator never makes one (`primitives.md` §6 clearance rule, per capsule height 1.2 m + 0.1 m).

## 3. Falling and death

**With gravity wells** the fall rule is measured along the local down and scaled by the field's strength (`gravity.md` §7). The numbers below are for standard gravity, the default field; a fall in another field accumulates the fall energy `e = Σ strength/256 · Δ(distance along down)` and dies at `e > 15 m`.

| Parameter | Value | Notes |
|---|---|---|
| Fall-death height | 15 m | measured from the height of the last ground contact. A 15 m fall lands at about 17 m/s after 1.75 s |
| Void timeout | 3.0 s of continuous falling with no ground found below within 200 m (raycast) | triggers death without waiting for a landing |
| Death sequence | the fall continues for 1.0 s after the trigger, then a 1.5 s fade to black, respawn, and a 1.5 s fade in | P2.5.3 |
| Drops under 15 m | harmless | they act as one-way shortcuts and don't affect connectivity |

## 4. Safe ground (respawn point)

The safe-ground position is a `WorldPos`, updated only while **all** of these hold:

1. The player has been grounded for at least 1.0 s.
2. The ground is a walkable surface (`primitives.md` §6).
3. **Edge probe:** 8 rays around the feet at a radius of 1.0 m each hit ground within the step-down distance (0.45 m). The player is not standing near a drop.
4. The player is not on a moving platform (lift car or tram).

On respawn the player is placed at that position, facing the direction they faced when it was recorded.

## 5. Transport

| Parameter | Value | Notes |
|---|---|---|
| Lift cruise speed | 30 m/s | One L1 cell (4096 m) takes about 2.5 min including acceleration. The ride is a set piece |
| Lift acceleration | 1.5 m/s² | gentle, no jolt |
| Short lift (L2 shafts, < 200 m) | 6 m/s | |
| Tram cruise speed | 100 m/s | 20 km in about 3.6 min: `d/V + V/A` = 200 s + 17 s, plus dwells |
| Tram acceleration | 6 m/s² | about 17 s (833 m) to cruise speed. Horizontal only, and the player is carried by the car's displacement, so it does not affect grounding. How the 0.6 g start and stop feel is a playtest item (`trams.md` §10) |
| Debug fly (noclip) | up to 1 km/s, adjustable | P2.4.5 |

## 6. Generator-facing dimensions

Derived from §1 and §2. The generator uses these (`levels.md`, `ports.md`), so changing them changes output.

| Dimension | Value | Derivation |
|---|---|---|
| Minimum clearance (headroom) | 2.4 m | capsule 1.8 m + 0.6 m |
| Minimum corridor or door width | 1.2 m | 2 × radius + 0.6 m |
| Walkway port minimum width | 2.0 m | `ports.md` §2 |
| Stair rise / tread | 0.1875 m / 0.3125 m (3 Q / 5 Q) | about 31°. The rise is well under the step-up |
| Walkway ramps | ≤ 15° | comfortable at walking pace. 40° is the hard limit |
| Railing height | 1.1 m | `levels.md` §5.4 |
| Stair tower rise / tread | 0.125 m / 0.3125 m (2 Q / 5 Q) | towers only (`levels.md` §5.2): a 4 m flight must divide the L2 cell, which 3 Q steps cannot |
| Guarded-edge threshold | drop > 15 m | equals the fall-death height |
| Narrowest walkable patch | must contain a disk of radius 0.3 m | `primitives.md` §6 |
| Crawl duct clear height | ≥ 1.3 m (crouch 1.2 m + 0.1 m) | §2.2; a duct under 1.9 m makes the player crouch |
| Ladder | rung spacing 0.30 m, clear width 0.5–0.8 m, landing ≥ 1.2 × 1.2 m, rest every ≤ 30 m | §2.2 |
| Walkable "up" | the local `up` of `gravity.md` §1; slopes and step-up are measured against it, and a surface is not walkable where the field strength is below 64/256 | `gravity.md` §6 |
