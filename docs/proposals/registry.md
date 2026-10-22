# Idea registry for the Web

Every idea in `the-web.md` ("web") and `the-web-ideas.md` ("ideas") has a stable ID here. This is how Phase W makes sure nothing is lost.

## Rules

1. **Every ID is owned by exactly one `CHECKLIST.md` item**, through that item's `Covers:` list. `scripts/coverage.py` checks this, and `scripts/check.sh` runs it. It also checks that each idea's text still appears in the doc it cites, so the docs and the registry cannot drift apart silently.
2. **An item is done only when every ID it covers is done.** For an ID, "done" means all three of:
   - it is specified in `docs/spec/`;
   - it exists in the world: the census (`web-report --census`) counts at least one instance in the sample region, or, for an engine-side idea, a test exercises it;
   - it was seen in an inspected screenshot or capture listed in the item's tick note.
3. **Cutting an idea needs the user.** To drop or defer an ID, ask the user, then move it to the "Deferred" table at the end with the date and reason. Never delete a row silently.
4. **Adding an idea means a new row and an owner.** A new idea in either proposal doc gets a new ID here (the next free number in its group; numbers are never reused) and goes on an item's `Covers:` list, or onto a new item.
5. **Generator output carries the IDs.** Primitives, zones and modules made for an idea carry its ID as a tag, which is what the census counts.

The "Idea" column is a phrase copied from the source doc, matched case-insensitively with markdown emphasis ignored.

## Principles (owned by the phase exit)

| ID | Idea | Source |
|---|---|---|
| PRN-01 | Built is the default; air is carved | web §1 |
| PRN-02 | Breathing space shows the extent | web §1 |
| PRN-03 | Every axis, and every angle, is a direction of travel | web §1 |
| PRN-04 | Everything crosses everything | web §1 |
| PRN-05 | No dead end without a payoff | web §1 |
| PRN-06 | The same grammar at every scale | web §1 |
| PRN-07 | Variety from combination | web §1 |
| PRN-08 | Not every combination is allowed | ideas §15.1 |

## Regime fields and orientation policy

| ID | Idea | Source |
|---|---|---|
| REG-01 | Fill, weave (bias towards axes and angles) | web §3 |
| REG-02 | grain (size scale) | web §3 |
| REG-03 | palettes of forms, supports and path types | web §3 |
| REG-04 | orientation lattice | web §3 |
| ORI-12 | axis-aligned (the calm default) | web §7.2 |
| ORI-13 | one yaw per L1 cell | web §7.2 |
| ORI-14 | one yaw per L2 cell (grains) | web §7.2 |
| ORI-15 | yaw and pitch (tilted strata) | web §7.2 |
| ORI-16 | radial around a well | web §7.2 |

## Stratum character

| ID | Idea | Source |
|---|---|---|
| SC-01 | Each stratum draws its character once | ideas §1 |
| SC-02 | 2–4 dominant quarters | ideas §1 |
| SC-03 | weights over forms, supports and path types | ideas §1 |
| SC-04 | grain and orientation policy | ideas §1 |
| SC-05 | scale (×½ to ×4 on the tissue) | ideas §1 |
| SC-06 | fill within the crust range | ideas §1 |
| SC-07 | material and palette weights | ideas §1 |
| SC-08 | light level and lamp density | ideas §1 |
| SC-09 | mutation weights | ideas §1 |
| SC-10 | Stratum seams | ideas §1 |
| SC-11 | Section cliffs show the characters stacked | ideas §1 |

## Strata, Betweens and crust surfaces

| ID | Idea | Source |
|---|---|---|
| STR-01 | a stack of strata | web §4.1 |
| STR-02 | separated by a Between 0.5–4 km deep | web §4.1 |
| STR-03 | columns 50–800 m wide spanning crust to crust | web §4.1 |
| STR-04 | inverted districts hanging from the crust above | web §4.1 |
| STR-05 | districts rising from the crust below | web §4.1 |
| STR-06 | a drip and a spire that meet halfway | web §4.1 |
| STR-07 | platforms stranded on a single tether | web §4.1 |
| STR-08 | transit tubes and trams | web §4.1 |
| STR-09 | cable nets | web §4.1 |
| STR-10 | dead machines the size of districts | web §4.1 |
| STR-11 | the far crust underside lit by window speckle | web §4.1 |
| BTW-01 | Shallow: 300 m | web §4.1 |
| BTW-02 | Abyssal: 4 km | web §4.1 |
| BTW-03 | Pillared: a hypostyle of tethers | web §4.1 |
| BTW-04 | Webbed: cables and catwalks only | web §4.1 |
| BTW-05 | Flooded with fog | web §4.1 |
| BTW-06 | Tilted: crusts slanted | web §4.1 |
| BTW-07 | Vertical: the strata are walls | web §4.1 |
| BTW-08 | Curved: strata wrapped concentrically | web §4.1 |
| SURF-01 | Roofscape: rooftops, antenna forests | web §4.1 |
| SURF-02 | Ceiling-scape: coffered undersides | web §4.1 |
| SURF-03 | Section cliff: where a crust is cut by a fracture | web §4.1 |
| SURF-04 | Skin: a near-blank shell | web §4.1 |

## Voids and breathing space

| ID | Idea | Source |
|---|---|---|
| VOID-01 | Valley: 300–2000 m wide | web §4.2 |
| VOID-02 | Hall: low and wide | web §4.2 |
| VOID-03 | Variants: hypostyle forest | web §4.2 |
| VOID-04 | hypostyle forest, vault | web §4.2 |
| VOID-05 | vault, cathedral, crush | web §4.2 |
| VOID-06 | crush (wide but 20 m tall) | web §4.2 |
| VOID-07 | stepped (terraced floor) | web §4.2 |
| VOID-08 | drowned in fog | web §4.2 |
| VOID-09 | Shaft: vertical, through a whole crust | web §4.2 |
| VOID-10 | Rift: a crack | web §4.2 |
| VOID-11 | Bore: a perfectly straight tunnel | web §4.2 |
| VOID-12 | Lungs: halls spaced regularly | web §4.2 |
| VOID-13 | Funnel: a conical void | web §4.2 |
| VOID-14 | Cavity: the rare event, as now | web §4.2 |
| LOC-01 | Light well, ringed by galleries | web §4.3 |
| LOC-02 | Atrium | web §4.3 |
| LOC-03 | Thin spot: the crust is only 50 m thick | web §4.3 |
| LOC-04 | Cut-away: a building with its façade missing | web §4.3 |
| LOC-05 | Courtyard sunk into a block | web §4.3 |
| LOC-06 | Gap between two leaning towers | web §4.3 |
| NEAR-01 | Grilles | web §4.4 |
| NEAR-02 | Slit views | web §4.4 |
| NEAR-03 | A doorway framing a far light | web §4.4 |
| NEAR-04 | Gaps in grating floors that show the drop | web §4.4 |
| NEAR-05 | - Windows | web §4.4 |

## Frame, fill and arteries

| ID | Idea | Source |
|---|---|---|
| FRM-01 | A beam is keyed by a line | web §5.1 |
| FRM-02 | Beams can now be oblique | web §5.1 |
| FRM-03 | square, flattened or round | web §5.1 |
| FRM-04 | ending in a broken stub over a void | web §5.1 |
| FRM-05 | solid girder | web §5.1 |
| FRM-06 | inhabited beam | web §5.1 |
| FRM-07 | truss: open lattice members | web §5.1 |
| FRM-08 | conduit: a bundle of giant pipes | web §5.1 |
| FRM-09 | rail beam: carries a tram | web §5.1 |
| FRM-10 | stepped beam | web §5.1 |
| FRM-11 | stair beam: an oblique beam | web §5.1 |
| FRM-12 | fallen beam | web §5.1 |
| FRM-13 | Knots: where beams meet inside a cell | web §5.1 |
| FRM-14 | a beam spanning a valley is a mega-bridge | web §5.1 |
| FILL-01 | Every L2 cell gets a fill density | web §5.3 |
| ART-01 | biased 3D random walk over the 8×8×8 L2 grid | web §5.4 |
| ART-02 | deterministic BFS route finishes it | web §5.4 |
| ART-03 | The ports and the knot form a tree, with loops added by chance | web §5.4 |
| ART-04 | grand stair, ramp road | web §5.4 |
| ART-05 | ramp road, lift trunk | web §5.4 |
| ART-06 | lift trunk, avenue | web §5.4 |
| ART-07 | avenue, tram tube | web §5.4 |
| ART-08 | tram tube, catwalk web | web §5.4 |
| ART-09 | catwalk web, inhabited beam corridor | web §5.4 |
| ART-10 | inhabited beam corridor, ladder chain | web §5.4 |
| ART-11 | ladder chain, gravity gate | web §5.4 |
| ART-12 | gravity gate. The lift core | web §5.4 |
| ART-13 | The lift core stops being a column in every cell | web §5.4 |

## Tissue order and supports

| ID | Idea | Source |
|---|---|---|
| TIS-01 | Reserve parent voids, frame pieces and artery corridors | web §6.1 |
| TIS-02 | Route. Connect every port | web §6.1 |
| TIS-03 | Grow structures around and onto the routes | web §6.1 |
| TIS-04 | A route segment marked engulfable | web §6.1 |
| TIS-05 | Dress the edges and place a payoff on every dead end | web §6.1 |
| TIS-06 | That is accretion | web §6.1 |
| SUP-01 | on a beam, a roof, a knot | web §6.2 |
| SUP-02 | Hanging (floors normal inside) | web §6.2 |
| SUP-03 | bridge-building over a gap | web §6.2 |
| SUP-04 | horizontal skyscraper 300 m long | web §6.2 |
| SUP-05 | a ring around a trunk | web §6.2 |
| SUP-06 | cliff dwellings in a beam | web §6.2 |
| SUP-07 | gondola rooms | web §6.2 |
| SUP-08 | jenga stack | web §6.2 |
| SUP-09 | building in an atrium | web §6.2 |
| SUP-10 | interpenetrating at an angle | web §6.2 |
| SUP-11 | standing on a gravity well's surface | web §6.2 |
| SV-01 | on stilts over a route | web §6.2 |
| SV-02 | on a slope with stepped footings | web §6.2 |
| SV-03 | kissing a standing tower below | web §6.2 |
| SV-04 | ending in a viewing pod | web §6.2 |
| SV-05 | chandelier cluster | web §6.2 |
| SV-06 | broken off with floors exposed | web §6.2 |
| SV-07 | hung from cables | web §6.2 |
| SV-08 | swung off-plumb | web §6.2 |
| SV-09 | pillar building between two beams | web §6.2 |
| SV-10 | arch building with a route under it | web §6.2 |
| SV-11 | fallen tower lying diagonally across a void | web §6.2 |
| SV-12 | from both sides of a trunk | web §6.2 |
| SV-13 | sagging floor by floor | web §6.2 |
| SV-14 | jutting out at a slant | web §6.2 |
| SV-15 | half ring | web §6.2 |
| SV-16 | true helix climbing the trunk | web §6.2 |
| SV-17 | a façade with nothing behind it | web §6.2 |
| SV-18 | windows only | web §6.2 |
| SV-19 | a cable-hung platform town | web §6.2 |
| SV-20 | platform town; cages | web §6.2 |
| SV-21 | omnidirectional cluster | web §6.2 |
| SV-22 | leaning pile across an alley | web §6.2 |
| SV-23 | a town in a hall | web §6.2 |
| SV-24 | a district inside a hollow tether | web §6.2 |
| SV-25 | 300 m long; stepped | web §6.2 |
| SV-26 | free; kissing | web §6.2 |

## Structure forms and mutations

| ID | Idea | Source |
|---|---|---|
| FORM-01 | Slab tower | web §6.3 |
| FORM-02 | Ziggurat, upright or inverted | web §6.3 |
| FORM-03 | Needle | web §6.3 |
| FORM-04 | Tube tower: an atrium core | web §6.3 |
| FORM-05 | H-pair with skybridges | web §6.3 |
| FORM-06 | Comb | web §6.3 |
| FORM-07 | Cage: a frame with floors | web §6.3 |
| FORM-08 | Cluster: 5–30 fused boxes | web §6.3 |
| FORM-09 | Drum / silo | web §6.3 |
| FORM-10 | Terrace cliff | web §6.3 |
| FORM-11 | Gantry straddling a route | web §6.3 |
| FORM-12 | Machine: turbine, gear stack | web §6.3 |
| FORM-13 | Rack canyon | web §6.3 |
| FORM-14 | Tank farm | web §6.3 |
| FORM-15 | - Ruin | web §6.3 |
| FORM-16 | Rotunda with a radial plan | web §6.3 |
| FORM-17 | Twisted tower: each floor turned | web §6.3 |
| FORM-18 | Leaning tower | web §6.3 |
| FORM-19 | Dome, faceted | web §6.3 |
| FORM-20 | Polygonal ring | web §6.3 |
| FORM-21 | Starburst hub | web §6.3 |
| FORM-22 | Wedge building | web §6.3 |
| FORM-23 | Cone or funnel tower | web §6.3 |
| FORM-24 | Roofed ring | web §6.3 |
| MUT-01 | (any form): inverted | web §6.3 |
| MUT-02 | inverted, tilted | web §6.3 |
| MUT-03 | tilted, twisted | web §6.3 |
| MUT-04 | twisted, broken | web §6.3 |
| MUT-05 | broken, overgrown | web §6.3 |
| MUT-06 | doubled or mirrored | web §6.3 |
| MUT-07 | mirrored, stretched | web §6.3 |
| MUT-08 | stretched, hollowed | web §6.3 |
| MUT-09 | hollowed, caged | web §6.3 |
| MUT-10 | pierced (a route bored through) | web §6.3 |
| MUT-11 | bored through), dark | web §6.3 |
| MUT-12 | dark, lit, scaled | web §6.3 |
| MUT-13 | scaled (×4 or ×¼) | web §6.3 |
| MUT-14 | fused with a neighbour | web §6.3 |
| MUT-15 | Swallowed: a mutation, not an era | ideas §1 |

## Paths

| ID | Idea | Source |
|---|---|---|
| PH-01 | catwalk: grating, 1.2–3 m | web §6.4 |
| PH-02 | skybridge: enclosed | web §6.4 |
| PH-03 | cable-stayed footbridge | web §6.4 |
| PH-04 | catenary bridge | web §6.4 |
| PH-05 | zigzag bridge | web §6.4 |
| PH-06 | beam-top walk | web §6.4 |
| PH-07 | pipe-top walk | web §6.4 |
| PH-08 | ledge walk | web §6.4 |
| PH-09 | façade gallery | web §6.4 |
| PH-10 | arcade through a row of buildings | web §6.4 |
| PH-11 | bored tunnel | web §6.4 |
| PH-12 | crawl duct | web §6.4 |
| PH-13 | grand avenue | web §6.4 |
| PH-14 | alley: a 2–6 m canyon floor | web §6.4 |
| PH-15 | roof walk across mismatched rooftops | web §6.4 |
| PH-16 | pier ending over a void | web §6.4 |
| PH-17 | gangway hung on chains | web §6.4 |
| PH-18 | dead conveyor | web §6.4 |
| PH-19 | skew corridor that crosses the grid | web §6.4 |
| PH-20 | curved gallery inside a rotunda | web §6.4 |
| PV-01 | straight flight | web §6.4 |
| PV-02 | switchback | web §6.4 |
| PV-03 | scissor stair | web §6.4 |
| PV-04 | square spiral round a pillar or a well | web §6.4 |
| PV-05 | true helical stair | web §6.4 |
| PV-06 | helical ramp | web §6.4 |
| PV-07 | fire escape on a façade | web §6.4 |
| PV-08 | cascade: 200 m down the face | web §6.4 |
| PV-09 | stair in a wall's thickness | web §6.4 |
| PV-10 | grand stair: 60–150 m wide | web §6.4 |
| PV-11 | stair-bridge that rises as it crosses | web §6.4 |
| PV-12 | stair beam (§5.1) | web §6.4 |
| PV-13 | wrap round a trunk | web §6.4 |
| PV-14 | terraces / amphitheatre | web §6.4 |
| PV-15 | inclined plane: a kilometre-long tilted slab | web §6.4 |
| PV-16 | lifts: freight, cage, open platform | web §6.4 |
| PV-17 | paternoster | web §6.4 |
| PV-18 | - escalator | web §6.4 |
| PV-19 | funicular: an inclined lift up a stair beam | web §6.4 |
| THR-01 | bulkhead hatch | web §6.4 |
| THR-02 | portal frame | web §6.4 |
| THR-03 | giant gate with a person-sized door | web §6.4 |
| THR-04 | broken wall | web §6.4 |
| THR-05 | floor hole with a rubble stair | web §6.4 |
| THR-06 | airlock pair | web §6.4 |
| THR-07 | climb-through window | web §6.4 |
| THR-08 | threshold of light | web §6.4 |
| THR-09 | gravity gate (§8.3) | web §6.4 |
| MOD-01 | - width | web §6.4 |
| MOD-02 | enclosure: open, railed, caged, walled, glazed, tunnel | web §6.4 |
| MOD-03 | condition: intact, missing rails | web §6.4 |
| MOD-04 | light: dark, lamp row | web §6.4 |
| MOD-05 | ceiling: open, crushing 2.2 m | web §6.4 |
| MOD-06 | rhythm: frames, arches or ribs | web §6.4 |
| MOD-07 | - material | web §6.4 |
| MOD-08 | dressing: cables, pipes overhead, signage glyphs | web §6.4 |
| MOD-09 | angle: on-grid, off-grid | web §6.4 |

## Places

| ID | Idea | Source |
|---|---|---|
| HUB-01 | junction hall | web §6.5 |
| HUB-02 | - hypostyle forest | web §6.5 |
| HUB-03 | machine cathedral | web §6.5 |
| HUB-04 | stair cathedral | web §6.5 |
| HUB-05 | reservoir hall | web §6.5 |
| HUB-06 | server canyon | web §6.5 |
| HUB-07 | valley rim | web §6.5 |
| HUB-08 | tram terminus | web §6.5 |
| HUB-09 | Between overlook | web §6.5 |
| HUB-10 | - gravity furnace | web §6.5 |
| HUB-11 | - sanctuary | web §6.5 |
| SET-01 | atrium with galleries | web §6.5 |
| SET-02 | - light well | web §6.5 |
| SET-03 | - cistern | web §6.5 |
| SET-04 | organ room | web §6.5 |
| SET-05 | window wall | web §6.5 |
| SET-06 | observation pod | web §6.5 |
| SET-07 | hanging garden | web §6.5 |
| SET-08 | niche / shrine | web §6.5 |
| SET-09 | generator room | web §6.5 |
| SET-10 | abandoned dwellings with human traces | web §6.5 |
| SET-11 | interchange: four routes crossing | web §6.5 |
| SET-12 | lift lobby | web §6.5 |
| SET-13 | tram stop | web §6.5 |
| SET-14 | valley balcony | web §6.5 |
| SET-15 | grain seam plaza | web §6.5 |
| SET-16 | tether foot | web §6.5 |
| RULE-01 | Dead-end rule. | web §6.5 |
| RULE-02 | Legibility. | web §6.5 |

## Free orientation

| ID | Idea | Source |
|---|---|---|
| ORI-01 | Grain: regions have their own grid rotation | web §7.1 |
| ORI-02 | fallen towers as bridges | web §7.1 |
| ORI-03 | - tilted strata | web §7.1 |
| ORI-04 | oblique struts and braces | web §7.1 |
| ORI-05 | stair beams and funiculars | web §7.1 |
| ORI-06 | rotundas, drums and domes | web §7.1 |
| ORI-07 | true helices | web §7.1 |
| ORI-08 | - catenaries | web §7.1 |
| ORI-09 | - polygonal rings | web §7.1 |
| ORI-10 | Radial: whole districts arranged around a gravity well | web §7.1 |
| ORI-11 | diagonal routes across a hall | web §7.1 |
| TEC-01 | Integer orientation. | web §7.3 |
| TEC-02 | Clipping instead of shape rules. | web §7.3 |
| TEC-03 | oriented box, oriented ramp, oriented stair run, cylinder on any axis | web §7.3 |
| TEC-04 | faceted helix built from those | web §7.3 |
| TEC-05 | transform the capsule into the primitive's frame | web §7.3 |
| TEC-06 | walk.rs is generalised from axis-aligned plates to oriented ones | web §7.3 |
| TEC-07 | The SDF of an oriented box is the axis-aligned SDF in its frame | web §11.5 |

## Gravity

| ID | Idea | Source |
|---|---|---|
| WELL-01 | Point well | web §8.1 |
| WELL-02 | Shell well | web §8.1 |
| WELL-03 | Line well | web §8.1 |
| WELL-04 | Tube well | web §8.1 |
| WELL-05 | Plane well | web §8.1 |
| WELL-06 | Twin wells | web §8.1 |
| WELL-07 | flips across a membrane | web §8.1 |
| WELL-08 | along a corridor's surface | web §8.1 |
| WELL-09 | A furnace failing: debris frozen in mid-air | web §8.1 |
| WELL-10 | inwards, weakening outward | web §8.1 |
| GRV-01 | is the local anti-gravity at its anchor | web §8.2 |
| GRV-02 | Strata wrap into concentric shells | web §8.2 |
| GRV-03 | Arteries run radially or along shells | web §8.2 |
| GRV-04 | Gravity gates: marked thresholds | web §8.3 |
| GRV-05 | a stair that turns over at its top | web §8.3 |
| GRV-06 | a ladder: you climb up, and at the top you are walking | web §8.3 |
| GRV-07 | a bridge whose far half belongs to the other well | web §8.3 |
| GRV-08 | Varieties: - a door | web §8.3 |
| GRV-09 | Gradients: inside a well the field turns gradually | web §8.3 |
| GRV-10 | Lulls: where two wells cancel | web §8.3 |
| GRV-11 | Falling: the fatal fall is measured along the local down | web §8.3 |
| GRV-12 | g(p) is a pure function of position | web §8.4 |
| GRV-13 | The controller is generalised from +Y to a local up vector | web §8.4 |
| GRV-14 | The free-drift verb is a slow push along the view direction | web §13 |

## Climbing

| ID | Idea | Source |
|---|---|---|
| CLB-01 | Starting: walk into a ladder while facing it | web §9.1 |
| CLB-02 | Moving: forward moves you in the direction you are looking | web §9.1 |
| CLB-03 | Ends: reaching the top or bottom steps you off | web §9.1 |
| CLB-04 | No falls: stepping off mid-ladder is not possible | web §9.1 |
| CLB-05 | Speed: a proposed 0.8 m/s | web §9.1 |
| CLB-06 | Rests: long ladders have a rest cage | web §9.1 |
| CLB-07 | auto-crouch: in a low duct the capsule shrinks to 1.2 m | web §9.1 |
| LAD-01 | Wall ladder | web §9.2 |
| LAD-02 | Caged ladder | web §9.2 |
| LAD-03 | Ladder well: through hatch after hatch | web §9.2 |
| LAD-04 | Ship's ladder at 60–70° | web §9.2 |
| LAD-05 | Staples set in concrete | web §9.2 |
| LAD-06 | Rungs welded up a pipe | web §9.2 |
| LAD-07 | Chain ladder that sways | web §9.2 |
| LAD-08 | Pillar climb: 200 m up a tether | web §9.2 |
| LAD-09 | Ladder beside a dead lift | web §9.2 |
| LAD-10 | Ladder chain: short ladders and ledges | web §9.2 |
| LAD-11 | Ladder down into a hanging tower's roof hatch | web §9.2 |
| LAD-12 | Ladder between balconies on a façade | web §9.2 |
| LAD-13 | Helical rungs round a drum | web §9.2 |
| LAD-14 | Gravity-gate ladder (§8.3) | web §9.2 |
| LAD-15 | Rung ladder across the ceiling of a lull tube | web §9.2 |

## L3 interiors

| ID | Idea | Source |
|---|---|---|
| L3-01 | any number of ForcedPorts on any face and storey | web §10.1 |
| L3-02 | including roof hatches, bottom hatches for hanging forms, and ladder ports | web §10.1 |
| L3-03 | the skeleton targets every entrance | web §10.1 |
| L3-04 | office grid | web §10.1 |
| L3-05 | cell block | web §10.1 |
| L3-06 | hall with mezzanine | web §10.1 |
| L3-07 | ramp core | web §10.1 |
| L3-08 | stacked lofts | web §10.1 |
| L3-09 | open cage | web §10.1 |
| L3-10 | machine floors | web §10.1 |
| L3-11 | rack aisles | web §10.1 |
| L3-12 | radial plan for rotundas | web §10.1 |
| L3-13 | twisted floors for twisted towers | web §10.1 |
| L3-14 | galleries over double-height rooms | web §10.1 |
| L3-15 | stepped corridors that climb half a storey | web §10.1 |
| L3-16 | enfilades of rooms in a line | web §10.1 |
| L3-17 | dog-legs with a window at the bend | web §10.1 |
| L3-18 | corridors open to an atrium | web §10.1 |
| L3-19 | skew corridors at the grain angle | web §10.1 |
| L3-20 | ladder wells between floors | web §10.1 |
| L3-21 | keep normal floors relative to their own down | web §10.1 |
| INT-01 | Room of stairs | ideas §9 |
| INT-02 | Hall of doors | ideas §9 |
| INT-03 | Cubicle field | ideas §9 |
| INT-04 | Parking helix | ideas §9 |
| INT-05 | Lift lobby of forty | ideas §9 |
| INT-06 | Ward: a hundred identical beds | ideas §9 |
| INT-07 | Auditorium | ideas §9 |
| INT-08 | Stadium hall | ideas §9 |
| INT-09 | Server cathedral: racks 30 m tall | ideas §9 |
| INT-10 | Cistern of pillars | ideas §9 |
| INT-11 | Bunk hall | ideas §9 |
| INT-12 | The Vessel | ideas §9 |
| INT-13 | Shrinking corridor | ideas §9 |
| INT-14 | Dining hall | ideas §9 |
| INT-15 | Map room | ideas §9 |
| INT-16 | Glass room | ideas §9 |
| INT-17 | Clock room | ideas §9 |
| INT-18 | Shower hall, laundry hall, kitchen | ideas §9 |
| INT-19 | Mirror corridor | ideas §9 |

## Kit and the staircase

| ID | Idea | Source |
|---|---|---|
| KIT-01 | window strips and fields | web §10.2 |
| KIT-02 | ledges and cornices | web §10.2 |
| KIT-03 | - ribs | web §10.2 |
| KIT-04 | vents and fans | web §10.2 |
| KIT-05 | - masts | web §10.2 |
| KIT-06 | façade pipes | web §10.2 |
| KIT-07 | cable runs (visual only | web §10.2 |
| KIT-08 | coffered undersides and secondary beams | web §10.2 |
| KIT-09 | hanging lamps on chains | web §10.2 |
| KIT-10 | glyph panels | web §10.2 |
| KIT-11 | stair railings | web §10.2 |
| KIT-12 | - scaffolds | web §10.2 |
| KIT-13 | every ladder family in §9.2 | web §10.2 |
| KIT-14 | gravity-gate rings | web §10.2 |
| KIT-15 | furnace machinery | web §10.2 |
| STAIR-01 | rebuilt from open flights (treads on stringers with air beneath) | web §10.2 |
| STAIR-02 | wound round a central core: glazed, lattice, or the hollow inside of a trunk | web §10.2 |
| STAIR-03 | it can also be a real round spiral | web §10.2 |
| STAIR-04 | The far band draws its core and stringer silhouette | web §10.2 |

## Horizon promise

| ID | Idea | Source |
|---|---|---|
| HOR-01 | Far things are a fixed set of L0/L1 features | web §11.1 |
| HOR-02 | Large features are L1 data. | web §11.2 |
| HOR-03 | L2 may only add what the far view cannot resolve. | web §11.2 |
| HOR-04 | The skin must actually be built. | web §11.2 |
| HOR-05 | Lights match. | web §11.2 |
| HOR-06 | far-vs-near test | web §11.2 |
| HOR-07 | A distinct far signature | web §11.3 |
| HOR-08 | A hub behind the signature | web §11.3 |
| HOR-09 | A route: arteries always lead to it | web §11.3 |
| HOR-10 | Landmarks are spaced so that one is usually in view from any overlook | web §11.3 |
| HOR-11 | Near breathing space comes often | web §11.4 |
| HOR-12 | A long view comes now and then | web §11.4 |
| HOR-13 | The Between overlook is rare | web §11.4 |
| HOR-14 | visible ways to make the trip shorter | web §11.4 |
| HOR-15 | occlusion culling hides most of the tissue | web §11.5 |

## Rare events (the-web.md)

| ID | Idea | Source |
|---|---|---|
| RARE-01 | Inverted district: everything hangs | web §12 |
| RARE-02 | Repeating corridor | web §12 |
| RARE-03 | Scale break: tissue at ×8 | web §12 |
| RARE-04 | Dollhouse: tissue at ×½ | web §12 |
| RARE-05 | Hollow trunk with a ramp helix inside | web §12 |
| RARE-06 | Builder site: half-built frame | web §12 |
| RARE-07 | The web: a void crossed only by catwalks | web §12 |
| RARE-08 | Lamp sun: one enormous light at a knot | web §12 |
| RARE-09 | Tram graveyard. | web §12 |
| RARE-10 | Planetoid: a point well | web §12 |
| RARE-11 | Möbius corridor: a fold well | web §12 |
| RARE-12 | Shattered grain: grains of 8 m | web §12 |
| RARE-13 | Section cliff: a fracture showing 1000 floors | web §12 |
| RARE-14 | Quiet lull: a weightless chamber | web §12 |

## Measurement

| ID | Idea | Source |
|---|---|---|
| MET-01 | Built fraction in crusts | web §14 |
| MET-02 | Built fraction in Betweens | web §14 |
| MET-03 | Median sightline from walkable points in crusts | web §14 |
| MET-04 | to a sightline of 2 km or more (median) | web §14 |
| MET-05 | Walking distance to near breathing space | web §14 |
| MET-06 | Crust skin covered by L2 | web §14 |
| MET-07 | Share of walked distance that is vertical | web §14 |
| MET-08 | Distinct path types per km walked | web §14 |
| MET-09 | Dead ends without a payoff | web §14 |
| MET-10 | Nav nodes with three or more links | web §14 |
| MET-11 | web-report --census | web §14 |

## Wonders

| ID | Idea | Source |
|---|---|---|
| WON-01 | The Cenotaph | ideas §2 |
| WON-02 | Tatlin's Spiral | ideas §2 |
| WON-03 | The Dam: a curved wall | ideas §2 |
| WON-04 | The Walking City | ideas §2 |
| WON-05 | The Tower of Babel, inverted | ideas §2 |
| WON-06 | The Organ: a cluster of pipes | ideas §2 |
| WON-07 | The Lamp Sun: a light so big | ideas §2 |
| WON-08 | The Loom | ideas §2 |
| WON-09 | The Monument: a perfectly blank slab | ideas §2 |
| WON-10 | The Hyperboloid | ideas §2 |
| WON-11 | The Wheel: a ring 2 km across | ideas §2 |
| WON-12 | The Cistern: a hall of 10 000 identical columns | ideas §2 |
| WON-13 | The Drop: a featherfall shaft 30 km deep | ideas §2 |
| WON-14 | The Library: hexagonal galleries | ideas §2 |
| WON-15 | The Bridge City | ideas §2 |
| WON-16 | The Nest | ideas §2 |
| WON-17 | The Stair: one grand stair 200 m wide | ideas §2 |
| WON-18 | The Furnace Heart | ideas §2 |
| WON-19 | The Antenna | ideas §2 |
| WON-20 | The Archive Wall | ideas §2 |
| WON-21 | The Mirror: a district duplicated | ideas §2 |
| WON-22 | The Sleeping Builder | ideas §2 |
| WON-23 | The Funnel: a conical void 8 km wide | ideas §2 |
| WON-24 | The Gate: a door in a Rampart | ideas §2 |

## Roads, Spines, Rivers

| ID | Idea | Source |
|---|---|---|
| RD-01 | Great Road: a lit processional route | ideas §3 |
| RD-02 | the Lantern Road | ideas §3 |
| RD-03 | the Dark Road | ideas §3 |
| RD-04 | the Pilgrim Road | ideas §3 |
| RD-05 | the Freight Road | ideas §3 |
| RD-06 | Lamp posts every 50 m, rest niches every 2 km, a sanctuary every 10 km | ideas §3 |
| RD-07 | Rail Abyss: a tram line across a Between | ideas §3 |
| RD-08 | Cable Highway: a gondola line | ideas §3 |
| SPN-01 | Spine: a vertical super-artery | ideas §3 |
| SPN-02 | a lift (a 30 minute ride | ideas §3 |
| SPN-03 | a stair that never ends | ideas §3 |
| SPN-04 | stations at every crust surface | ideas §3 |
| RIV-01 | River: a pipe bundle 300–800 m wide | ideas §3 |
| RIV-02 | the hot river | ideas §3 |
| RIV-03 | the dead river | ideas §3 |
| RIV-04 | the broken river | ideas §3 |

## Quarters

| ID | Idea | Source |
|---|---|---|
| QTR-01 | Dwelling warren | ideas §4 |
| QTR-02 | Foundry | ideas §4 |
| QTR-03 | Data | ideas §4 |
| QTR-04 | Transit | ideas §4 |
| QTR-05 | Archive | ideas §4 |
| QTR-06 | Hydroponic | ideas §4 |
| QTR-07 | Necropolis | ideas §4 |
| QTR-08 | Administrative | ideas §4 |
| QTR-09 | Bastion | ideas §4 |
| QTR-10 | Dock | ideas §4 |
| QTR-11 | Waterworks | ideas §4 |
| QTR-12 | Observatory | ideas §4 |
| QTR-13 | Market | ideas §4 |
| QTR-14 | Chapel | ideas §4 |
| QTR-15 | Hospital | ideas §4 |
| QTR-16 | Machine | ideas §4 |

## Forms, batch 2

| ID | Idea | Source |
|---|---|---|
| F2-01 | Capsule tower | ideas §5 |
| F2-02 | Habitat stack | ideas §5 |
| F2-03 | Arcology pyramid | ideas §5 |
| F2-04 | Cooling tower: a hyperboloid shell | ideas §5 |
| F2-05 | Dish: a radio dish | ideas §5 |
| F2-06 | Gasometer | ideas §5 |
| F2-07 | Blast furnace: a cone | ideas §5 |
| F2-08 | Viaduct: arches marching | ideas §5 |
| F2-09 | Plug-in megaframe | ideas §5 |
| F2-10 | Space-frame canopy | ideas §5 |
| F2-11 | Metabolist trunk | ideas §5 |
| F2-12 | Stepped slab | ideas §5 |
| F2-13 | Boullée hall | ideas §5 |
| F2-14 | Ledoux sphere house | ideas §5 |
| F2-15 | Cathedral: a nave | ideas §5 |
| F2-16 | Monster building | ideas §5 |
| F2-17 | Battleship island | ideas §5 |
| F2-18 | Bridge house | ideas §5 |
| F2-19 | Silo bank | ideas §5 |
| F2-20 | Lift tower cluster | ideas §5 |
| F2-21 | Stair tower field | ideas §5 |
| F2-22 | Hollow mountain | ideas §5 |
| F2-23 | Hanging garden tower | ideas §5 |
| F2-24 | Lattice ghost | ideas §5 |

## Paths, batch 2

| ID | Idea | Source |
|---|---|---|
| P2-01 | pipe interior | ideas §6 |
| P2-02 | gutter: a channel | ideas §6 |
| P2-03 | track walk | ideas §6 |
| P2-04 | tram-car bridge | ideas §6 |
| P2-05 | chain walk | ideas §6 |
| P2-06 | colonnade walk | ideas §6 |
| P2-07 | cornice walk | ideas §6 |
| P2-08 | in-wall passage | ideas §6 |
| P2-09 | cable-bundle tunnel | ideas §6 |
| P2-10 | glass-floored bridge | ideas §6 |
| P2-11 | covered market street | ideas §6 |
| P2-12 | ceiling walk | ideas §6 |
| P2-13 | rolling ladder on an archive wall | ideas §6 |
| P2-14 | dumbwaiter shaft | ideas §6 |
| P2-15 | chimney climb | ideas §6 |
| P2-16 | stair-escalator | ideas §6 |
| P2-17 | counterweight pair | ideas §6 |
| P2-18 | Ferris cabins | ideas §6 |
| P2-19 | bucket chain: a slow loop | ideas §6 |
| P2-20 | funicular on a stair beam | ideas §6 |
| P2-21 | Spiral of Spirals | ideas §6 |
| P2-22 | grand ramp: a helix 200 m | ideas §6 |
| P2-23 | stepped pyramid stair | ideas §6 |
| P2-24 | inverted stair | ideas §6 |
| P2-25 | tether climb | ideas §6 |
| P2-26 | moving bridge | ideas §6 |
| P2-27 | drawbridge | ideas §6 |
| P2-28 | lift that turns | ideas §6 |
| P2-29 | tram that goes through a gravity gate | ideas §6 |
| P2-30 | Builder ride: a slow giant machine | ideas §6 |

## Journeys

| ID | Idea | Source |
|---|---|---|
| RIDE-01 | Spine lift | ideas §7 |
| RIDE-02 | Featherfall descent down the Drop | ideas §7 |
| RIDE-03 | Tram across a Rail Abyss | ideas §7 |
| RIDE-04 | Gondola on the Cable Highway | ideas §7 |
| RIDE-05 | The Wheel | ideas §7 |
| RIDE-06 | A Builder ride | ideas §7 |
| RIDE-07 | Drift across the Archipelago | ideas §7 |
| RIDE-08 | Bucket chain | ideas §7 |
| RIDE-09 | Grand Stair of the Stair | ideas §7 |
| RIDE-10 | Following a River | ideas §7 |

## Weightless and low gravity

| ID | Idea | Source |
|---|---|---|
| WGT-01 | Featherfall shafts: shafts with weak gravity | ideas §8 |
| WGT-02 | straight, helical, branching, with platforms to drift between, upward (a rising current) | ideas §8 |
| WGT-03 | Drift lanes | ideas §8 |
| WGT-04 | Gravity rivers: a long visible current | ideas §8 |
| WGT-05 | The Archipelago: a lull the size of a Between | ideas §8 |
| WGT-06 | Tumbler room | ideas §8 |
| WGT-07 | Orbiters | ideas §8 |
| WGT-08 | Low-gravity districts | ideas §8 |
| WGT-09 | Weightless cathedral | ideas §8 |

## Payoffs, batch 2

| ID | Idea | Source |
|---|---|---|
| PAY-01 | Anamorphic glyph | ideas §10 |
| PAY-02 | Cartographer's room | ideas §10 |
| PAY-03 | Focal window | ideas §10 |
| PAY-04 | The previous walker | ideas §10 |
| PAY-05 | A lit room in a dark district | ideas §10 |
| PAY-06 | Collections: a room of 400 lamps | ideas §10 |
| PAY-07 | Messages: glyph inscriptions | ideas §10 |
| PAY-08 | Echo chamber | ideas §10 |
| PAY-09 | A seat: sometimes it is just a bench | ideas §10 |
| PAY-10 | The bell: a bell in a shaft | ideas §10 |

## Light

| ID | Idea | Source |
|---|---|---|
| LGT-01 | Lighthouses: knot towers | ideas §11 |
| LGT-02 | Searchlights sweeping | ideas §11 |
| LGT-03 | Light rivers | ideas §11 |
| LGT-04 | Aurora sheets | ideas §11 |
| LGT-05 | Window weather | ideas §11 |
| LGT-06 | smaller lamp suns over halls | ideas §11 |
| LGT-07 | Furnace glow lighting fog from below | ideas §11 |
| LGT-08 | God rays through a perforated crust | ideas §11 |
| LGT-09 | Blackout strata | ideas §11 |
| LGT-10 | Emergency light: red lamps | ideas §11 |
| LGT-11 | Neon glyph signage | ideas §11 |

## Weather and air

| ID | Idea | Source |
|---|---|---|
| WX-01 | Fog sea | ideas §12 |
| WX-02 | Fog tides | ideas §12 |
| WX-03 | Condensation rain | ideas §12 |
| WX-04 | Dust fall | ideas §12 |
| WX-05 | Ash snow | ideas §12 |
| WX-06 | Steam vents | ideas §12 |
| WX-07 | Wind in bores and Roads | ideas §12 |
| WX-08 | Frost near cold machinery | ideas §12 |
| WX-09 | Heat haze | ideas §12 |
| WX-10 | Lightning: arcs along conduits | ideas §12 |
| WX-11 | Still pools: coolant mirrors | ideas §12 |

## Presence

| ID | Idea | Source |
|---|---|---|
| BLD-01 | You can watch them place a beam over an hour | ideas §13.1 |
| BLD-02 | You can ride one | ideas §13.1 |
| BLD-03 | You can find a dock where one is half-built | ideas §13.1 |
| BLD-04 | Far away their work lights are slow-moving stars | ideas §13.1 |
| BLD-05 | schedule of predetermined geometry | ideas §13.1 |
| NOT-01 | an engine hall that starts turning as you walk through | ideas §13.2 |
| NOT-02 | lift cars that come when you approach | ideas §13.2 |
| NOT-03 | doors opening ahead of you | ideas §13.2 |
| LIFE-01 | Drone flocks wheeling | ideas §13.4 |
| LIFE-02 | Maintenance crawlers on pipes | ideas §13.4 |
| LIFE-03 | Cable spiders mending nets | ideas §13.4 |
| LIFE-04 | Dust moths around lamps | ideas §13.4 |
| LIFE-05 | a single lantern light on a distant catwalk | ideas §13.5 |
| GROW-01 | moss on north faces | ideas §13.6 |
| GROW-02 | hanging roots in wells | ideas §13.6 |
| GROW-03 | pale fungus that glows | ideas §13.6 |
| GROW-04 | vines in Hydroponic quarters | ideas §13.6 |
| GROW-05 | a whole tree of pipes that is actually a tree | ideas §13.6 |

## Quiet tools and multipliers

| ID | Idea | Source |
|---|---|---|
| TOOL-01 | A held zoom | ideas §14.1 |
| TOOL-02 | gaze-mark, pin a far landmark | ideas §14.1 |
| TOOL-03 | Telescope rooms in Observatory quarters | ideas §14.1 |
| TOOL-04 | Named places, auto-recorded on arrival | ideas §14.2 |
| TOOL-05 | with a transliteration | ideas §14.2 |
| TOOL-06 | The pages: the Atlas shows each recorded place | ideas §14.2 |
| TOOL-07 | Hold still to hear a direction | ideas §14.3 |
| TOOL-08 | Draw a mark on a wall | ideas §14.4 |
| MUL-01 | A procedural glyph alphabet, fixed per seed | ideas §15.2 |
| MUL-02 | No two hubs within 30 km share the same combination | ideas §15.3 |
| SYS-01 | World-clock movers | ideas §19 |

## Strange regions

| ID | Idea | Source |
|---|---|---|
| STG-01 | The Library (§2.14), as a whole region | ideas §16 |
| STG-02 | The Recursion | ideas §16 |
| STG-03 | The Inversion | ideas §16 |
| STG-04 | The Grid: a lattice of identical 64 m cubes | ideas §16 |
| STG-05 | Field of Doors | ideas §16 |
| STG-06 | The Labyrinth | ideas §16 |
| STG-07 | Fog district | ideas §16 |
| STG-08 | Silent district | ideas §16 |
| STG-09 | The Slow district | ideas §16 |
| STG-10 | The Miniature | ideas §16 |
| STG-11 | The Far Lands | ideas §16 |
| STG-12 | The Rampart | ideas §16 |
| STG-13 | Builder frontier | ideas §16 |

## Sanctuaries, batch 2

| ID | Idea | Source |
|---|---|---|
| SAN-01 | Tea room over a Between | ideas §17 |
| SAN-02 | Greenhouse under a small lamp sun | ideas §17 |
| SAN-03 | Library nook | ideas §17 |
| SAN-04 | - Lamp chapel | ideas §17 |
| SAN-05 | Bunk room with a radio murmuring | ideas §17 |
| SAN-06 | Observatory: a telescope room | ideas §17 |
| SAN-07 | Needle top | ideas §17 |
| SAN-08 | Gondola cabin parked at a tower | ideas §17 |
| SAN-09 | Pool room | ideas §17 |
| SAN-10 | Bell room | ideas §17 |
| SAN-11 | Garden in a light well | ideas §17 |
| SAN-12 | Hanging pod under a crust | ideas §17 |
| SAN-13 | Some are reached only by a long ladder or a featherfall | ideas §17 |

## Sound (design; audio deferred)

| ID | Idea | Source |
|---|---|---|
| SND-01 | The City's breath | ideas §18 |
| SND-02 | Horns: a tram horn | ideas §18 |
| SND-03 | Stress: beams groaning | ideas §18 |
| SND-04 | Wind organs | ideas §18 |
| SND-05 | The Organ Wonder, heard from 20 km | ideas §18 |
| SND-06 | Bells in shafts, falling sound | ideas §18 |
| SND-07 | Choir resonance | ideas §18 |
| SND-08 | Sanctuary chimes | ideas §18 |
| SND-09 | Silence zones | ideas §18 |

## Deferred

Ideas the user chose to drop or postpone, with the date and reason. They keep their IDs and are not checked for coverage.

| ID | Idea | Date | Reason |
|---|---|---|---|
| NOT-04 | a line of lamps lighting up a kilometre ahead | 2026-10-22 | The user: "remove the lamp spawning, it doesn't suit and is annoying" (PW.13.0). |
| WAKE-01 | Dead lights wake as you pass | 2026-10-22 | The user: "remove the lamp spawning, it doesn't suit and is annoying" (PW.13.0). |
| WAKE-02 | your own trail | 2026-10-22 | The user: "remove the lamp spawning, it doesn't suit and is annoying" (PW.13.0). |

Note: "eras" (eight period styles by depth) were dropped by the user on 2026-10-18 before this registry existed. Stratum character (SC-xx) replaces them.
