# Spec: traversal

**Status: draft, awaiting user approval of the numbers (P0.2.8).**

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
| Jump | **none** (proposed, §2.1) | |

### 2.1 Why no jump (proposal)

- **Zen means no friction** (CLAUDE.md §9). Without a jump there is no platforming and no failed jumps.
- **The connectivity spec assumes it.** Ports, step-up and stairs define what is reachable. A jump would make places reachable that the generator never connected, and would invite falls.
- **Small obstacles are handled by step-up.** Step-up (0.45 m) and stairs cover everything the generator builds.

If a jump is wanted later, a small hop (0.5 m apex) is the conservative choice. It would not change any generator dimension.

## 3. Falling and death

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
| Tram cruise speed | 100 m/s | 20 km in about 3.5 min |
| Tram acceleration | 1.5 m/s² | about 67 s to cruise speed |
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
| Guarded-edge threshold | drop > 15 m | equals the fall-death height |
| Narrowest walkable patch | must contain a disk of radius 0.3 m | `primitives.md` §6 |
