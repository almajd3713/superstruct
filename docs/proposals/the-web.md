# Proposal: the Web (generator redesign)

*Status: proposal, 2026-10-18, revision 2. It replaces the content of L1 and L2, changes L3, and adds free orientation, gravity wells and climbing to the engine. The hashing, quantized coordinates, streaming, renderer, lifts, trams, sanctuaries and the L3 solver stay.*

## Decisions taken (user, 2026-10-18)

| Question | Answer |
|---|---|
| The Web direction (§1–§6) | Yes |
| Axis-aligned only (decided 2026-10-17) | **Reversed:** diagonals and free angles too (§7) |
| Ladders and climbing | Yes (§9) |
| Gravity | Not a flip. **Gravity wells** in the spirit of Toha Heavy Industries, where every direction can be down (§8) |
| Density | As dense as possible, but with breathing space that shows the extent of the structure, near and far, like the gap between old and new Aperture in Portal 2 (§4) |
| Horizon promise in a dense world | Moments of awe need not be frequent. The promise is "you can get to that far structure, and you will find something new there" (§11) |
| Ladder speed, crouch | 0.8 m/s climbing as proposed; all speeds are tunable later |
| Gravity well frequency | About one every 50 km |
| Gravity gate transition | A smooth camera roll |
| Lulls | Both handrail tubes and free drifting |
| Iteration 3 | More ideas without limit inside the scope: the idea bank `docs/proposals/the-web-ideas.md` |

## 0. What it should feel like

You come off a spiral stair onto a grating catwalk slung under the belly of a tower that hangs upside down from a beam. You can't see its top or the ground. The catwalk runs into the side of a horizontal tower that grows out of a trunk at a slant. You enter it on what its lift lobby calls floor 14. The floor is normal, with ordinary corridors and rooms. One of those rooms has lost its outer wall, and through it you see a canyon two kilometres wide. Threads cross it at forty heights and a dozen angles: a fallen tower lying across it like a log, beam-bridges, cable footbridges, a tram tube, a single pipe with a walkway on top. You climb down a caged ladder through six floors of hatches. Then you leave by a door onto the roof of the building below, which is a street. From there you can take a stair cascading down the face of the next block, a paternoster still clanking, or a crawl duct.

The duct ends at a window, and beyond the window there is no floor. Two kilometres below is the roof of the next stratum. Two kilometres above is the underside of the one you are in. Between them hang inverted cities, tethers as wide as towns, lights crawling along a transit tube, and the whole layered structure receding into haze in every direction. That is the Between.

Far across it there is a glow where the strata bend round something. Closer up, the buildings point at it from every side. That is a gravity furnace, and the district around it is a small world of its own.

## 1. Principles

1. **Built is the default; air is carved.** Inside a stratum, 70–90 % of the volume is structure.
2. **Breathing space shows the extent.** The density is interrupted at every scale, so you can see how much of it there is (§4).
3. **Every axis, and every angle, is a direction of travel.** Climbing and descending make up at least a third of the distance walked. Near a gravity well, "down" itself turns.
4. **Everything crosses everything.** Structures pierce, straddle, hang from, lean on and collide with each other at angles. Paths go through buildings, not around them.
5. **No dead end without a payoff.** A branch that stops must stop at something: a view, a set piece, a light, a sanctuary, a lift.
6. **The same grammar at every scale.** Strata, L1 arteries, L2 routes and L3 rooms are all walks through containers, so the world is self-similar by construction.
7. **Variety from combination.** Every idea below is a family with parameters and variants, combined with supports, path types, orientations and mutations. None is a single fixed shape.

## 2. Why the current generator can't get there

It works by addition: L1 floats 0–3 deck slabs per 4 km layer, and L2 sets a few boxes on the slabs. A 4 km vertical cross-section is two thin lines with specks on them. Turning up the numbers gives more boxes on the same slabs. Inside, L3 builds a tree from one door, so every building is a cul-de-sac. Everything is also axis-aligned, under one fixed down. The topology has to change, not the density.

## 3. The levels, reassigned

| Level | Becomes |
|---|---|
| L0 Regime (32 km) | **Strata**: their orientation, thickness and Between gaps (§4.1). **Gravity wells** (§8). Fill, weave (bias towards axes and angles), grain (size scale), the **orientation lattice** (grid yaw and grain size, §7.2), and palettes of forms, supports and path types |
| L1 (4 km) | **Frame** (beams on any axis or slant, knots), **Voids**, the **Fill field**, and **Arteries** (random walks over the 8×8×8 L2 cells) |
| L2 (512 m) | **Tissue**: routes first, then structures grown on every kind of support, at the local grid angle, then set pieces |
| L3 (64 m) | Interiors with many entrances on many floors that connect every entrance to every other, so buildings are through-routes |
| L4 (8 m) | Interior and exterior kit, ladders, and the props of every path type |

## 4. Density and breathing space

The rule: **thick, dense crusts, separated by gaps that let you see them.** Breathing space happens at four scales, and each comes in several kinds.

### 4.1 Macro: strata and the Between

The world is a stack of **strata**. Each stratum is a **crust** of tissue 1–6 km thick, and each pair of crusts is separated by a **Between** 0.5–4 km deep. In the Between you see crust surfaces receding to the horizon, so it is where the extent of the megastructure becomes visible. These are the layers of BLAME!'s Megastructure, and the space between old and new Aperture.

**A Between is never empty:**
- **tethers:** columns 50–800 m wide spanning crust to crust, some hollow with ramps inside;
- **drips:** inverted districts hanging from the crust above;
- **spires:** districts rising from the crust below;
- **kisses:** a drip and a spire that meet halfway, with a waist of stairs;
- **islands:** platforms stranded on a single tether;
- **transit tubes and trams;**
- **cable nets;**
- **dead machines** the size of districts;
- **the far crust underside lit by window speckle,** like an inverted night city.

**Between variants:**
- **Shallow:** 300 m. Ceiling-scape and roof-scape almost touch, and a forest of tethers fills the gap.
- **Abyssal:** 4 km. The far crust is a faint pattern of lights.
- **Pillared:** a hypostyle of tethers on a grid, receding forever.
- **Webbed:** cables and catwalks only, no tethers.
- **Flooded with fog:** visibility drops to 300 m, and lights are the only landmarks.
- **Tilted:** crusts slanted 5–30°, so the Between is a wedge that narrows to a seam far away.
- **Vertical:** the strata are walls. The Between is a canyon 1–4 km wide, endless in height and length, crossed by bridges at every height.
- **Curved:** strata wrapped concentrically around a gravity well (§8).

**Crust surface variants:**
- **Roofscape:** rooftops, antenna forests, terraces.
- **Ceiling-scape:** coffered undersides, drips, hanging lamps, pipes.
- **Section cliff:** where a crust is cut by a fracture, a wall of exposed floors, hundreds of them, with lit windows. This is density seen from outside.
- **Skin:** a near-blank shell with sparse openings, as in the Megastructure walls in BLAME!.

### 4.2 Meso (km)

- **Valley:** 300–2000 m wide and tens of km long, keyed by a line, so it continues across cells.
- **Hall:** low and wide. Variants: hypostyle forest, vault, cathedral, crush (wide but 20 m tall), stepped (terraced floor), drowned in fog.
- **Shaft:** vertical, through a whole crust, so you can see the Between at its end.
- **Rift:** a crack that steps or slants through a crust.
- **Bore:** a perfectly straight tunnel 10–50 km long. Standing in it, you see a dot of light at the far end, which is the horizon promise along a line.
- **Lungs:** halls spaced regularly, like alveoli, connected by bores.
- **Funnel:** a conical void narrowing down into a crust, possible now that angles are allowed.
- **Cavity:** the rare event, as now.

### 4.3 Local (100 m)

- **Light well**, ringed by galleries
- **Atrium**
- **Thin spot:** the crust is only 50 m thick, so a window opens onto the Between
- **Cut-away:** a building with its façade missing, so you see into every floor
- **Courtyard** sunk into a block
- **Gap between two leaning towers**

### 4.4 Near (10 m)

- Windows
- Grilles
- Slit views
- A doorway framing a far light
- Gaps in grating floors that show the drop

**Targets:** crust 70–90 % built; Between 2–10 % built. The regime moves the balance. Pipeworks has thick crusts and shallow Betweens. Hollow Expanse has thin crusts and abyssal Betweens.

## 5. L1: Frame, Voids, Fill, Arteries

### 5.1 Frame: beams on every axis and angle

A beam is keyed by a line, the way a trunk is keyed by its column now. Every cell the line crosses derives the same beam and emits its own clipped piece (§7.3). Trunks become the vertical case of the same thing. Beams can now be **oblique**: struts and braces at any angle, a truss at megascale.

- **Section:** 32–400 m, square, flattened or round. Weave biases axes and angles.
- **Extent:** the full span, or ending at a knot, or ending in a broken stub over a void.
- **Kinds** (each with variants):
  - **solid girder:** walkable on top and an underside to hang things from;
  - **inhabited beam:** a hollow tube with floors and tissue inside;
  - **truss:** open lattice members you walk through;
  - **conduit:** a bundle of giant pipes;
  - **rail beam:** carries a tram;
  - **stepped beam:** offset boxes drifting up or down;
  - **stair beam:** an oblique beam at ≤ 35° with a stair cut along its top, 2 km of climbing in one straight line;
  - **fallen beam:** a beam lying at an angle across a void, as if dropped.
- **Knots:** where beams meet inside a cell there is a junction structure. A knot is a hub: a junction hall, a lift nexus, a machine, a gravity furnace or a sanctuary site. It is what the arteries aim for.

Frame members cross voids and Betweens, so a beam spanning a valley is a mega-bridge for free.

### 5.2 Voids

Voids are the meso and macro spaces of §4.1–§4.2, emitted as `KeepOut`s. Tissue never fills them; the frame and threads may cross them.

### 5.3 Fill field

Every L2 cell gets a fill density. It is trilinear in integer weights k/8 between values hashed at L1 cell corners, so neighbours agree and it is a pure function. Strata set the mean and voids set it to zero. L2 fills its cell to within ±10 % of the density, and its outer skin covers the far envelope (§11).

### 5.4 Arteries: random walks at megascale

Each L1 face carries ports, derived from face keys as now; a port is an L2 cell on that face. Inside the cell, each port gets a **biased 3D random walk over the 8×8×8 L2 grid** to the cell's knot:

- the moves are the six axis steps plus diagonal steps, weighted by weave and pulled towards the target;
- a walk never revisits a cell and steps over voids only as a spanning type;
- if the walk does not arrive within a bounded number of steps, a deterministic BFS route finishes it.

The ports and the knot form a tree, with loops added by chance, so every port is connected by construction. Each artery has a **type**, which becomes ForcedPorts on the L2 faces it crosses: grand stair, ramp road, lift trunk, avenue, tram tube, catwalk web, inhabited beam corridor, ladder chain, gravity gate. The lift core stops being a column in every cell and becomes one artery type; the backbone guarantee moves to the arteries.

## 6. L2: the Tissue

### 6.1 Order (it is what makes connectivity hold)

1. **Reserve** parent voids, frame pieces and artery corridors.
2. **Route.** Connect every port, artery end and set-piece anchor with walks on the local slot lattice, which is rotated to the local grid angle (§7.2). The lattice is still almost empty at this point, so a route always exists.
3. **Grow structures** around and onto the routes until the cell reaches its fill density. A route segment marked *engulfable* may end up inside a structure. It then becomes ForcedPorts on that structure, and L3 must connect them. That is where multiple entrances on multiple floors come from.
4. **Dress** the edges and place a payoff on every dead end.

Each structure adds new supports (roof, underside, sides), so later structures grow on earlier ones. That is accretion, the way the Kowloon Walled City grew: floors cantilevered over alleys until they reached the building opposite.

### 6.2 Supports

| Support | Variants |
|---|---|
| **Standing** | on a beam, a roof, a knot, on stilts over a route, on a slope with stepped footings |
| **Hanging** (floors normal inside) | free; **kissing** a standing tower below, joined by a stair; ending in a viewing pod; a **chandelier** cluster; broken off with floors exposed; hung from cables; swung off-plumb |
| **Spanning** | **bridge-building** over a gap; **pillar building** between two beams; **arch building** with a route under it; **fallen tower** lying diagonally across a void |
| **Sideways** | a **horizontal skyscraper** 300 m long; stepped; from both sides of a trunk; sagging floor by floor; jutting out at a slant |
| **Collar** | a ring around a trunk; half ring; a **true helix** climbing the trunk |
| **Embedded** | cliff dwellings in a beam; a façade with nothing behind it; windows only |
| **Suspended** | gondola rooms; a cable-hung platform town; cages |
| **Piled** | jenga stack; omnidirectional cluster; leaning pile across an alley |
| **Nested** | building in an atrium; a town in a hall; a district inside a hollow tether |
| **Colliding** | two structures from different grid grains interpenetrating at an angle, with the wedge where they meet becoming a space (§7.2) |
| **Radial** | standing on a gravity well's surface, pointing away from it (§8) |

### 6.3 Structure forms

Any form can be inverted, turned sideways, tilted or rotated where it makes sense.

- **Slab tower**
- **Ziggurat**, upright or inverted. Inverted, every step is a walkable terrace.
- **Needle**
- **Tube tower**: an atrium core
- **H-pair** with skybridges at several heights
- **Comb**
- **Cage**: a frame with floors, no walls
- **Cluster**: 5–30 fused boxes
- **Drum / silo**, upright, hanging or lying down
- **Terrace cliff**
- **Gantry** straddling a route
- **Machine**: turbine, gear stack, piston bank, radiator fins
- **Rack canyon**
- **Tank farm**
- **Ruin**
- **Rotunda** with a radial plan
- **Twisted tower**: each floor turned a few degrees
- **Leaning tower**
- **Dome**, faceted
- **Polygonal ring**
- **Roofed ring**: a polygonal ring with a roof, flat or with an oculus; the plain ring stays open to the sky
- **Starburst hub** with bridges radiating at 45°
- **Wedge building** filling an angle between grains
- **Cone or funnel tower**

**Mutations** (any form): inverted, tilted, twisted, broken, overgrown, doubled or mirrored, stretched, hollowed, caged, pierced (a route bored through), dark, lit, scaled (×4 or ×¼), fused with a neighbour.

### 6.4 Paths: the circulation grammar

A route link is a sequence of **segments**. Each segment picks a type from the regime palette, filtered by context: a height change wants vertical types, a gap wants spanning types, an engulfable segment wants interior types, and a change in gravity wants a gravity gate. Modifiers are drawn independently, and nodes are small places: landings, platforms and junctions.

**Horizontal**
- catwalk: grating, 1.2–3 m, railed, half-railed or bare
- skybridge: enclosed, with windows or blind
- cable-stayed footbridge
- **catenary bridge** that sags between two towers
- **zigzag bridge**
- beam-top walk
- pipe-top walk
- ledge walk
- façade gallery
- arcade through a row of buildings
- bored tunnel
- crawl duct
- grand avenue
- alley: a 2–6 m canyon floor between blocks
- roof walk across mismatched rooftops
- pier ending over a void
- gangway hung on chains
- dead conveyor
- **skew corridor** that crosses the grid at an angle
- **curved gallery** inside a rotunda

**Vertical**
- straight flight
- switchback
- scissor stair
- square spiral round a pillar or a well
- **true helical stair**
- **helical ramp**
- fire escape on a façade
- cascade: 200 m down the face of the tissue, as in Chongqing
- stair in a wall's thickness
- grand stair: 60–150 m wide, Nihei-scale
- stair-bridge that rises as it crosses
- stair beam (§5.1)
- wrap round a trunk
- terraces / amphitheatre
- inclined plane: a kilometre-long tilted slab ≤ 25°, climbed slowly
- lifts: freight, cage, open platform
- **paternoster**
- escalator
- **funicular**: an inclined lift up a stair beam
- the ladders of §9

**Thresholds**
- bulkhead hatch
- portal frame
- giant gate with a person-sized door cut into it
- broken wall
- floor hole with a rubble stair
- airlock pair
- climb-through window
- threshold of light
- **gravity gate** (§8.3)

**Modifiers**, any of which applies to any segment:
- width
- enclosure: open, railed, caged, walled, glazed, tunnel
- condition: intact, missing rails, a missing section with a detour
- light: dark, lamp row, floor strip, flicker, one far light at the end
- ceiling: open, crushing 2.2 m, normal, cathedral
- rhythm: frames, arches or ribs every N m
- material
- dressing: cables, pipes overhead, signage glyphs
- angle: on-grid, off-grid by the local grain, free

### 6.5 Places: the payoffs

**L1 hubs:**
- junction hall
- hypostyle forest
- machine cathedral
- stair cathedral
- reservoir hall
- server canyon
- valley rim
- tram terminus
- Between overlook
- gravity furnace
- sanctuary

**L2 set pieces:**
- atrium with galleries
- light well
- cistern
- organ room
- window wall
- observation pod
- hanging garden
- niche / shrine
- generator room
- abandoned dwellings with human traces
- interchange: four routes crossing at four heights in one space
- lift lobby
- tram stop
- valley balcony
- **grain seam plaza**: a triangular space where two grid angles meet
- **tether foot**: where a tether lands on a roofscape

**Dead-end rule.** Dead-end branches must end at a set piece, a view (a sightline of at least 200 m through a void or the Between), a light or a lift. The validator checks this.

**Legibility.** Lit landmarks visible from routes (knot lamps, beacons, trams, a furnace glow) give the walker something to aim for. Arteries bias towards the knot, so following the big stair or the lamp leads to a hub.

## 7. Free orientation

### 7.1 Ideas

- **Grain:** regions have their own grid rotation, like crystal grains in a metal. Where grains meet, structures collide at angles. This makes wedge alleys, triangular plazas, skew bridges, and buildings sheared off by the neighbouring grain. Grain size goes from 4 km (calm) down to 64 m (chaotic), set by the regime.
- **Slants:**
  - leaning towers
  - fallen towers as bridges
  - tilted strata
  - oblique struts and braces
  - stair beams and funiculars
  - twisted towers
- **Curves:** made of faceted pieces, so collision stays analytic:
  - rotundas, drums and domes
  - true helices
  - curved galleries
  - catenaries
  - polygonal rings
- **Radial:** whole districts arranged around a gravity well (§8). Every structure's "up" points away from the furnace.
- **Off-grid paths:** skew corridors, zigzag bridges, starbursts, and diagonal routes across a hall.

### 7.2 Orientation lattice

Each L0 regime draws an orientation policy, and L1 and L2 cells inherit a **grid frame** from it. Every frame is an integer-keyed rotation. Some policies:
- axis-aligned (the calm default);
- one yaw per L1 cell;
- one yaw per L2 cell (grains);
- yaw and pitch (tilted strata);
- radial around a well.

### 7.3 How it stays exact and deterministic

- **Integer orientation.** A rotation is stored as integer direction vectors, for example a yaw as `(a, b)` from a fixed table of 64 headings, or an "up" as an integer vector. Bases are built with `sqrt`, division and cross products. Those are correctly rounded in IEEE 754, so the same integers give the same floats everywhere. No `sin` or `cos` ever runs in the generator; the table is a constant. Positions stay in Q. Invariant 3 (hash on integers only) holds.
- **Clipping instead of shape rules.** Every primitive carries its owning cell's box as a clip. Then `contains = shape.contains ∧ clip.contains`, `sdf = max(shape, clip)` (a valid bound), and meshing clips triangles to the box. Any keyed feature of any shape and angle can cross faces by emitting its own clipped piece. That keeps rule 1 of the ownership rule (`levels.md` §1.1) and drops the "only boxes clip" restriction.
- **New shapes:** oriented box, oriented ramp, oriented stair run, cylinder on any axis, and a faceted helix built from those.
- **Collision:** transform the capsule into the primitive's frame and run the existing axis-aligned test. Still analytic, still exact.
- **Navigation and validation:** walkability is judged against the local gravity (§8) and the 40° slope limit, on oriented plates. `walk.rs` is generalised from axis-aligned plates to oriented ones.

## 8. Gravity wells

A **gravity furnace** is a dead or dying machine, like the gravity reactor of Toha Heavy Industries in BLAME!. It bends "down" around itself, and the structure built near it obeys. The world is built so that every direction can be down.

### 8.1 Well kinds

| Kind | Down points… | What you get |
|---|---|---|
| **Point well** | towards a centre | A planetoid of megastructure: concentric shells of tissue around the furnace. Every building is radial, and you can walk around it and come back from the other side. The horizon curves away |
| **Shell well** | outward from a centre | The inside of a sphere. You stand anywhere on its inner wall, and the opposite side is overhead, 2 km up, lit like a sky |
| **Line well** | towards an axis | A beam or tether you can walk all the way round. Towers stand out from every side of it like bristles on a brush |
| **Tube well** | outward from an axis | The inside of a tube kilometres long. Districts line every wall; you look up and see streets upside down overhead |
| **Plane well** | along one direction | A sideways district: down is +x, so a valley wall is the floor, and normal towers grow horizontally out of it |
| **Twin wells** | towards the nearer of two furnaces | Two worlds facing each other, with a weightless lull between them |
| **Seam** | flips across a membrane | Climb a stair through a doorway and you are on the ceiling of the room you just left. Escher's *Relativity* as a place |
| **Fold** | along a corridor's surface | Walls become floors round a bend, the corridor keeps turning, and you end up walking the other way upside down (as in *Manifold Garden*) |
| **Damaged** | in patches | A furnace failing: debris frozen in mid-air at angles, pockets of different down, a slow pulse |
| **Tidal** | inwards, weakening outward | Gravity falls off with distance, so falls are slower and safer at the edge, and a fragment of a district floats free there |

### 8.2 How structures respond

Near a well, every structure's "up" is the local anti-gravity at its anchor, quantized to the integer direction table (§7.3), so a planetoid is faceted at large scale. Strata wrap into concentric shells. Arteries run radially or along shells. Ladders and lifts follow the local down.

### 8.3 Crossing between gravities

- **Gravity gates:** marked thresholds, such as a ring of light or a bulkhead, where down changes. The camera rolls smoothly to the new down, and the controller reorients at a capped rate. Varieties:
  - a door
  - a stair that turns over at its top
  - a ladder: you climb up, and at the top you are walking
  - a bridge whose far half belongs to the other well
- **Gradients:** inside a well the field turns gradually. Walking over a planetoid's curve, you never notice, and then look back.
- **Lulls:** where two wells cancel there is no down. Lulls are never walkable floor. They are crossed in enclosed tubes with a handrail that draws you along, which is the ladder mechanic laid flat.
- **Falling:** the fatal fall is measured along the local down and scaled by strength. In a weak field you can drop much further safely.

### 8.4 Generation and physics

`g(p)` is a pure function of position: the wells are keyed features of L0 and L1. The walk validator, respawn, the controller and the lift and tram systems all take "up" from it. The controller is generalised from +Y to a local up vector. The far bands are unchanged, since geometry is geometry.

## 9. Ladders and climbing

### 9.1 Mechanic

There is no new button:
- **Starting:** walk into a ladder while facing it and you are on it.
- **Moving:** forward moves you in the direction you are looking, up or down.
- **Ends:** reaching the top or bottom steps you off onto the landing.
- **No falls:** stepping off mid-ladder is not possible, so climbing is safe.
- **Speed:** a proposed 0.8 m/s, a player-feel number for you to approve.
- **Rests:** long ladders have a rest cage or platform every ≤ 30 m.

Climbing also allows **auto-crouch**: in a low duct the capsule shrinks to 1.2 m by itself, with a slower walk. That opens crawl ducts, vents, and gaps under machines.

### 9.2 Ladder families

- Wall ladder
- Caged ladder
- **Ladder well**: through hatch after hatch, floor after floor
- Ship's ladder at 60–70°
- Staples set in concrete
- Rungs welded up a pipe
- Chain ladder that sways (visual only)
- **Pillar climb**: 200 m up a tether with rest platforms, a long, slow, zen climb
- Ladder beside a dead lift in its shaft
- Ladder chain: short ladders and ledges zigzagging up a cliff (a via ferrata)
- Ladder down into a hanging tower's roof hatch
- Ladder between balconies on a façade
- Helical rungs round a drum
- Gravity-gate ladder (§8.3)
- Rung ladder across the ceiling of a lull tube

## 10. L3 and L4

### 10.1 L3: buildings as passages

- **Entrances:** a mass takes any number of ForcedPorts on any face and storey, including roof hatches, bottom hatches for hanging forms, and ladder ports.
- **Plan:** the skeleton targets every entrance, so the building is a route.
- **Interior archetypes**, chosen by form:
  - office grid
  - cell block
  - hall with mezzanine
  - ramp core
  - stacked lofts
  - open cage
  - machine floors
  - rack aisles
  - **radial plan** for rotundas
  - **twisted floors** for twisted towers
- **Corridor variety:**
  - galleries over double-height rooms
  - stepped corridors that climb half a storey
  - enfilades of rooms in a line
  - dog-legs with a window at the bend
  - corridors open to an atrium
  - skew corridors at the grain angle
  - ladder wells between floors
- **Inverted, sideways and radial forms** keep normal floors relative to their own down. Only their silhouettes and entrances differ.

### 10.2 L4: the outside gets dressed, and ladders get built

New kit:
- window strips and fields
- ledges and cornices
- ribs
- vents and fans
- masts
- façade pipes
- cable runs (visual only, so they may sag and curve)
- coffered undersides and secondary beams
- hanging lamps on chains
- glyph panels
- stair railings
- scaffolds
- every ladder family in §9.2
- gravity-gate rings
- furnace machinery

**The infinite staircase** is rebuilt from open flights (treads on stringers with air beneath), railings and landing beams, wound round a central core: glazed, lattice, or the hollow inside of a trunk. Now that true helices are allowed, it can also be a real round spiral. The far band draws its core and stringer silhouette rather than a block.

## 11. The horizon promise in a dense world

Density and long views pull against each other. Inside a crust, most sightlines end within tens or hundreds of metres. The design accepts that and changes what the promise means in practice.

> **You see that structure far away? With enough time you can get there, and when you do you will find something new.**

The moments of awe are **rare and earned**, not constant. These rules keep the promise true when they come.

### 11.1 Far things are a fixed set of L0/L1 features

Only these are visible at kilometres:
- crust surfaces and their large openings
- the Frame
- tethers, drips and spires
- knots and their landmarks
- voids
- trams and lights

All of them are L0/L1 data, drawn the same at every distance. Tissue is never seen far away except as part of a crust surface.

### 11.2 A crust surface is exact at the scale it is seen from

The risk is that a dense area is drawn far away as a solid block, and turns out porous up close. That would be the "far thing changes when you approach" bug. The fix is a size split:

- **Large features are L1 data.** L1 defines the crust's outer **skin**: its setbacks, overhangs and every opening wider than 32 m. These are drawn in the far band and realised exactly up close.
- **L2 may only add what the far view cannot resolve.** That means openings narrower than 32 m, and relief shallower than 16 m. At the 3 km far-band handover, 32 m is about 0.6°, a few pixels. Anything bigger has to come from L1.
- **The skin must actually be built.** L2 covers the skin to at least 90 %, not "stays inside the envelope". So the far silhouette is the near façade.
- **Lights match.** Window lights in the far band come from the same hash that lights the tissue's windows, so the pattern you see from 5 km is the one you walk past.

This is how the hierarchy already works for masses (L3 within the L2 envelope). It is now stated per feature size, with a check: a **far-vs-near test** renders a crust surface from 5 km and from 300 m and compares the silhouettes after downsampling.

### 11.3 Every landmark has a destination

Every L1 knot gets three things:
- **A distinct far signature:** a silhouette type, a light colour and rhythm, and sometimes a beacon.
- **A hub behind the signature:** something new when you arrive. A junction hall, a machine, a furnace, a sanctuary or a stair cathedral; never just more tissue.
- **A route:** arteries always lead to it. That is by construction (§5.4).

A tower you see from across a Between is therefore a knot you can identify, reach and find something in. Landmarks are spaced so that one is usually in view from any overlook.

### 11.4 How often the views come

- **Near breathing space** comes often, every few minutes of walking: atria, light wells, windows, cut-aways, thin spots.
- **A long view** comes now and then: a valley, a hall, a bore, or a shaft ending at the Between. Target: from anywhere, a sightline of 2 km or more is reachable within roughly 15–30 minutes of walking.
- **The Between overlook** is rare: each crossing between strata is an event.

From each overlook there are visible ways to make the trip shorter: trams across the Between, lifts down tethers, funiculars on stair beams.

### 11.5 Rendering

Oriented pieces are drawn the same way at every distance. The SDF of an oriented box is the axis-aligned SDF in its frame, so the horizon raymarch stays cheap. Dense crusts help performance: occlusion culling hides most of the tissue, and the expensive far views happen only in the Betweens, where there is little tissue to draw.

## 12. Rare and mind-breaking

These come on top of the cavity and the infinite staircase:

- **Inverted district:** everything hangs.
- **Repeating corridor:** one section repeated for a kilometre.
- **Scale break:** tissue at ×8, with 30 m doors.
- **Dollhouse:** tissue at ×½.
- **Hollow trunk** with a ramp helix inside.
- **Builder site:** half-built frame, scaffold, and one slowly moving builder machine.
- **The web:** a void crossed only by catwalks and cables.
- **Lamp sun:** one enormous light at a knot, visible for 10 km.
- **Tram graveyard.**
- **Planetoid:** a point well with a whole small world around it.
- **Möbius corridor:** a fold well that returns you upside down.
- **Shattered grain:** grains of 8 m, where every room is at a different angle.
- **Section cliff:** a fracture showing 1000 floors at once.
- **Quiet lull:** a weightless chamber crossed by a single handrail tube, with fog and one lamp.

## 13. Open questions

All four were answered on 2026-10-18 (see the decisions table): ladders at 0.8 m/s, wells about every 50 km, a smooth camera roll at gates, and both handrails and free drift in lulls. The free-drift verb is a slow push along the view direction at about 1 m/s; contact with any surface stops you gently, and a handrail pulls you along faster. Iteration 3 adds many more ideas in `docs/proposals/the-web-ideas.md`; the new systems it needs are listed in §16.

## 14. Measuring it

A `web-report` tool (extending `slice --report`) gives numbers to tune against before looking at screenshots:

| Metric | Target |
|---|---|
| Built fraction in crusts | 70–90 % |
| Built fraction in Betweens | 2–10 % |
| Median sightline from walkable points in crusts | 30–150 m |
| Walking distance from a random walkable point to a sightline of 2 km or more (median) | 1.5–2.5 km, about 15–30 min |
| Walking distance to near breathing space (atrium, well, window to a void) (median) | ≤ 150 m |
| Crust skin covered by L2 (far-vs-near rule, §11.2) | ≥ 90 % |
| Share of walked distance that is vertical (random bot walks) | ≥ 33 % |
| Distinct path types per km walked | ≥ 8 |
| Dead ends without a payoff | 0 |
| Nav nodes with three or more links | ≥ 40 % |

**Census.** Every catalogue entry is tagged in the generator output with its ID from `docs/proposals/registry.md`. Then `web-report --census` counts how many instances of each ID appear over a fixed sample region. A checklist item can therefore prove that its ideas exist in the world, not only in the code.

## 15. Order of work

Each step ends with a review: slices, report numbers, and in-game screenshots from fixed viewpoints.

1. **W0. Specs:**
   - `levels.md` §4–6
   - `ports.md` §6 (the new connectivity argument)
   - `primitives.md` (orientation, clip boxes)
   - `traversal.md` (ladders, crouch, gravity)
   - a new `gravity.md`
   - the PLAN.md decisions
   - `GEN_VERSION` 7
2. **E1. Oriented primitives:** shapes, clip boxes, SDF, meshing, collision, the nav generalisation. With axis-aligned frames, the output must be bit-identical to today.
3. **W1. L0/L1:** strata and Betweens, frame with oblique beams, voids, fill field, arteries.
4. **W2. L2:**
   - routes first, then structures (6 forms × 5 supports, 12 path types to start), then payoffs;
   - grains, step 2: one yaw per L1 cell first, then per L2 cell.
5. **W3. L3:** multi-entrance plans, new interior archetypes, ladder wells.
6. **E2. Climbing:** ladders, auto-crouch, handrail tubes.
7. **E3. Gravity:**
   - the `g(p)` field;
   - the controller on a local up;
   - gates and camera roll;
   - respawn, lifts and validation under any down;
   - the first well (a line well: the easiest to test, and striking).
8. **W4. Far bands and SDF horizon** for the frame, crust surfaces and envelopes; streaming radii; performance budget.
9. **W5. Exterior kit, ladder kit, and the staircase rebuild.**
10. **W6. Widen the catalogues,** regime by regime, towards everything above, and the rest of the gravity wells.

## 16. Iteration 3: the idea bank

`docs/proposals/the-web-ideas.md` adds:
- **Stratum character:** every stratum draws its own recipe from the shared catalogue (quarters, forms, paths, grain, scale, light, mutations), so crossing a Between lands you somewhere different without new asset sets. This replaces the "eras" of the first draft, which the user dropped as too asset-heavy.
- **Wonders:** about 24 unique far landmarks, one per 30–100 km.
- **Large-scale routes:** Great Roads, Spines and Rivers at L0 scale.
- **Neighbourhoods:** 16 quarters.
- **More content:** more forms, paths, interiors, payoffs and sanctuaries.
- **Long journeys and weightlessness:** rides, featherfall shafts, the Archipelago.
- **Light, weather and presence:** non-hostile Builders you can ride, and wake lights that leave your trail visible.
- **Quiet tools:** scope with gaze-mark, the Atlas with names in a procedural script, listening, chalk.
- **Strange regions,** and the sound design.

Its §19 lists the new systems; they are CHECKLIST group PW.10.

## Sources of ideas

- Kowloon Walled City: [Metropolis](https://metropolismag.com/projects/how-hong-kongs-vertical-village-came-be/), [National Geographic](https://www.nationalgeographic.com/history/article/kowloon-walled-city-history)
- Chongqing: [CNN via ABC17](https://abc17news.com/news/2025/08/05/the-mindbending-cyberpunk-city-in-china-that-exists-on-so-many-levels/)
- Piranesi, *Carceri*: [Cooper Hewitt](https://www-4.collection.cooperhewitt.org/objects/18425217), [Royal Academy](https://royalacademy.org.uk/art-artists/work-of-art/the-gothic-arch)
- Constant, *New Babylon*: [MACBA](https://www.macba.cat/en/node/27963), [Situationists](https://situationists.webflow.com/constant)
- Yona Friedman, *Ville Spatiale*: [MoMA](https://www.moma.org/collection/works/104695)
- Lebbeus Woods, *Underground Berlin*: [M+](https://mplus.org.hk/en/collection/objects/drawings-underground-berlin-2016831)
- *Gigastructure*: [itch.io](https://starmote.itch.io/gigastructure)
- *Manifold Garden*: [Kill Screen](https://www.killscreen.com/manifold-garden-turns-magnificent-architecture-endless-playground/)
- BLAME!: [Wikipedia](https://en.wikipedia.org/wiki/Blame!), [Kill Screen](https://www.killscreen.com/influence-blame-videogame-architecture-rising/), gravity furnace chapter recap ([Nihei wiki](https://tsutomu-nihei.fandom.com/wiki/LOG._27_-_Gravity_Furnace))
- Portal 2, the underground: [Valve developer wiki](https://developer.valvesoftware.com/wiki/Underground_(Portal_2))
