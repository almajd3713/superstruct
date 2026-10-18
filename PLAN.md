# Superstruct: research and plan

A first-person, zen exploration game set in a procedurally generated, unbounded (x/y/z) cyberpunk megastructure in the spirit of BLAME!. The only verbs are walking and exploring. Anything you can see, even on the horizon, can eventually be reached.

*Status: planning only (2026-10-17). Nothing built yet.*

---

## 1. What exists already

| Game | Procedural? | Notes |
|---|---|---|
| **NaissanceE** (2014, UDK) | No | The reference for a lonely, huge, BLAME!-flavoured space. It's a linear path with open areas. Critics found the platforming and puzzles awkward in it, so a pure-walking design is the right instinct. |
| **Megastructure** (Pierral46, UE5, 2024) | No | First person and heavily inspired by BLAME!. Mostly built from a bought asset pack. |
| **Sector №9 – Megastructure** | Yes | Riding a hoverbike through endless megastructures. In open beta. |
| **project_Structure_01** (Mar 2026) | No | Brutalist, liminal, first-person. 94% positive reviews. Shows there's an audience. |
| **BLACKSHARD**, **Metal Garden**, **Megastructure** (Scatter, third-person climbing, upcoming) | Mostly no | Same genre. There's a Steam curator just for "Megastructure Games". |
| **marian42's infinite WFC city** | Yes | Walking only, no gameplay. The closest technical cousin. |

**The gap:** no game combines procedural generation, unbounded 3D space, first-person walking and a promise that anything you see on the horizon can be reached. That promise is the hook, and it shapes most of the technical decisions below.

---

## 2. The central problem: what you see far away must be what you find

If a tower 20 km away must still be there, with the same shape, when you arrive, the generator needs three properties:

1. **Stateless and deterministic.** Content is a pure function of the seed and the cell coordinates. It never depends on the order things were generated or the path the player took. This rules out plain "infinite WFC with backtracking". marian42 reports late-detected failures, cases where the cell the player is standing in had to be regenerated, and calls the approach unsuitable for commercial games.
2. **Top-down hierarchy.** Coarse levels decide the big forms, and finer levels only refine inside what their parent decided. A finer level never contradicts its parent. No Man's Sky works the same way: the whole planet exists at coarse detail before you land.
3. **Local.** A cell depends only on its parents and a bounded set of neighbours, so streaming works in any direction, including straight up or down.

**What follows:** the far-away version of a structure comes from the coarse generator levels, not from simplifying fine meshes. You never have to generate the fine detail of something 20 km away just to draw it. This is what makes "unlimited" affordable.

### Proposed scale hierarchy (each level about 8× the next)

| Level | Cell size | Decides | Seen from |
|---|---|---|---|
| **L0 Regime** | ~32 km | Density (packed vs hollow), architectural era/style, light and fog palette, rare events | Everywhere, as atmosphere |
| **L1 Megaforms** | ~4 km | Trunks (pillars 200–800 m wide), deck slabs a kilometre across, shafts, chasms, giant cavities | Up to the horizon, 20–50 km |
| **L2 Districts** | ~512 m | Building masses, bridges between megaforms, stair towers, ramps, lifts; the walkable connection graph | 300 m – 3 km |
| **L3 Blocks** | ~64 m | Rooms, corridors, stairwells, pipe bundles (via bounded WFC, see below) | < 300 m |
| **L4 Modules** | ~4–8 m | Kit pieces: wall panels, railings, cables, lights, doors | Near |
| **L5 Dressing** | — | Props, decals, grime | Very near |

### Guaranteeing you can walk there

- **Ports on shared faces.** Each face of a cell gets ports (openings, bridge ends, stair landings) derived from a hash of the face's own ID. Both neighbouring cells compute the same ports independently, so they agree without ever communicating.
- **Each cell must connect all of its ports internally.** If every cell does that, connectivity across the whole world follows by induction.
- **Backbone lattice.** *Decided 2026-10-17:* every L1 face (4 km) carries at least one port, and every L1 column has a lift core (`docs/spec/ports.md`). This rules out unreachable islands at megastructure scale. Dead ends and sealed rooms remain at L2 and L3 for variety. It replaces the earlier idea of a sparse lattice (every 4th face).
- **Ports cascade down levels.** An L1 bridge ending in an L2 cell forces L2 ports, which force L3 boundary conditions.
- **WFC only inside fixed blocks.** Run WFC (or Merrell's model synthesis) only within a single L3 block whose boundary is already fixed by its ports. Failure is local: retry with a new sub-seed, then fall back to a known-good template. This is the same idea as Nested WFC (Nie et al. 2023) and Hierarchical Semantic WFC (Alaka & Bidarra 2023).

### Vertical travel is a real design problem

At a calm walking speed of 1.4 m/s, 4 km takes about 48 minutes, and a structure 20 km away is about 4 hours. Honouring "eventually reachable" needs ways to move faster that never cut to a loading screen:

- **Giant freight lifts and long spiral ramps.** Riding them can itself be a zen set piece.
- **Maglev trams along L1 decks.** At 50–100 m/s, 20 km takes about 4–7 minutes.
- **Falling** was decided in §10.3: falls over 15 m are fatal, and you respawn at the last safe ground. Numbers are in `docs/spec/traversal.md`. There is no jump.

---

## 3. Unlimited coordinates

- A 32-bit float has roughly 1 mm precision at 8 km and roughly 8 mm at 65 km, so jitter shows up quickly. Store positions as an **integer sector (int64) plus a float32 offset inside the sector**, and move the origin when the player leaves a sector (a "floating origin").
- **Compute every hash on integer cell coordinates.** Generation is then exact anywhere. int64 sectors of 1 km reach about 9×10¹⁸ km. BLAME!'s City is about 1.6×10⁹ km across (Nihei's own estimate).
- Use **reversed-Z depth** for view distances of 20–50 km.
- Engines already support this: Unreal 5 has Large World Coordinates (doubles), Godot has double-precision builds (you compile them yourself), and Bevy has the `big_space` crate (grid cells, i8 to i128).

---

## 4. Rendering at scale: layered far field

| Band | Distance | Representation |
|---|---|---|
| Near | 0–300 m | Full kit modules, instanced, with collision |
| Mid | 300 m – 3 km | Merged, simplified chunk meshes built from L2 data (HLOD), or Nanite-style virtual geometry if the engine has it |
| Far | 3–50 km | L1 megaforms only. These are simple shapes (boxes, cylinders, slabs), so they can be drawn as cheap instances generated from the same rules, which keeps them exactly consistent |
| Horizon | > 50 km | Silhouettes raymarched as signed distance fields (SDFs) from L0/L1 rules, or slowly refreshed baked panoramas, all fading into fog |

- **Atmosphere is the single biggest cue for scale.** Shadow of the Colossus relied on haze plus drawing even very distant scenery. Use height fog, aerial perspective and volumetric light shafts.
- **Transitions** between bands: dither or cross-fade. Silhouettes match because they come from the same data.
- **Octahedral impostors** suit repeated far assets, but they cast no correct shadows. In fog and darkness that matters less.
- **Lighting.** You can't bake lighting into geometry generated at runtime. Either use real-time GI (Lumen), or rely on emissive surfaces, fog, AO and a budget of local lights. BLAME!'s dark look with glowing panels happens to suit the cheaper option well.

---

## 5. Streaming

- Nested shells of chunks around the player, one shell per hierarchy level (like clipmaps).
- Generation runs on worker threads and is prioritized by view direction and distance, within a fixed per-frame time budget.
- Chunks live in an LRU cache. **Nothing needs saving**, because everything is deterministic, which suits a game with no editing.
- Collision exists only for near chunks. Size the budget for the worst case: a tram running at 100 m/s.

---

## 6. Engine choice

| Engine | For | Against |
|---|---|---|
| **Unreal 5.7** | Nanite, Lumen, Large World Coordinates, volumetric fog, World Partition. The PCG framework is out of beta and supports runtime generation | C++, a heavy engine. Nanite meshes generated at runtime are awkward, but that's fine if you **instance pre-made Nanite kit modules**, which suits a kit-bash style anyway |
| **Godot 4.6** | Open source, light, Jolt physics by default, generator can be written in Rust or C++ via GDExtension | No virtual geometry; you build doubles yourself; lighting at this scale is your job |
| **Unity 6** | Burst/Jobs are excellent for generation, HDRP volumetrics | Floating origin is manual |
| **Bevy** | Rust/ECS, `big_space`, fun to work in | Meshlets are still experimental, tooling is thin, more engine work |
| **Custom (WebGPU/Vulkan)** | Full control over GPU-driven and SDF rendering | Most work |

**Decided:** a custom engine in Rust + wgpu, with the generator as an engine-agnostic library. See §10 and §11.

---

## 7. Design for "zen, sometimes mind-breaking"

- **Scale needs human-sized references.** Railings, doors, stairs and lamps up close make the distance readable. Thousands of identical windows also read as size.
- **Slow motion far away.** Lights crawling along distant structures, a Builder robot, a tram the size of a town read as scale through how slowly they move.
- **Compression then release.** A narrow duct opens onto a kilometre-wide void. L2 can schedule this rhythm on purpose.
- **Guaranteed vistas.** A generator rule that sometimes frames a view: a window onto an abyss, an overhang above a cavity.
- **Rare L0 events** for the mind-breaking moments:
  - a cavity the size of a planet (BLAME!'s Jupiter room)
  - an infinite staircase
  - regions where gravity changes
  - self-similarity, where the same grammar repeats at every scale
- **Sound:**
  - reverb sized to the space around the player (probe it)
  - a distant machinery drone
  - echoing footsteps
- **Light, untimed tools:**
  - an altimeter and coordinates showing absurd numbers
  - beacons you can drop
  - a photo mode

---

## 8. Main risks and mitigations

| Risk | Mitigation |
|---|---|
| Monotony ("10,000 bowls of oatmeal") | Regimes, landmarks, rare events, compression/release rhythm |
| Unreachable spots, getting stuck, bad falls | Ports-by-construction plus **automated pathfinding bots** that walk generated cells in CI |
| Far and near versions don't match | One rule set drives every LOD band, with automated checks that far silhouettes match near geometry |
| Hitches from generation at tram speed | Per-frame budgets, predictive prefetch along the tram line |
| Lighting procedural geometry | Lumen, or a deliberate emissive-and-fog art style |
| Scope creep | The staged plan below, with exit criteria |

---

## 9. Plan

0. **Specification and generator.** Write the generator spec (levels, ports, regimes) and the `gen` crate, plus a slice viewer that outputs PNG cross-sections. *Exit:* port connectivity is proven on 10⁶ random cells, and generation is deterministic across runs and threads.
1. **Engine skeleton.** winit + wgpu window, camera-relative rendering, reversed-Z depth, egui debug overlay, Tracy profiling, WGSL shader hot reload. *Exit:* a static greybox scene renders, with GPU timings visible.
2. **Greybox walker.** Stream L1 and L2 primitives around the player. Add the first-person controller with analytic collision, fog, safe-ground respawn and floating origin. *Exit:* walk to one horizon structure and confirm it matches what you saw from afar. **Answer the key question: does walking in it already feel good?**
3. **GPU-driven pipeline and far field.** Shared mesh buffers, compute frustum and Hi-Z occlusion culling, multi-draw-indirect, the LOD bands, the SDF horizon pass, lifts and trams. *Exit:* 60 fps at 1440p on the target GPU while riding at 100 m/s, with no popping.
4. **Detail.** L3 bounded WFC, L4 kit modules (generated in code at first), first art-direction and stylization pass.
5. **Atmosphere and audio.** Froxel volumetric fog, clustered lights, sanctuaries, regimes, landmarks, rare events, reverb probes.
6. **Polish.** Performance, test bots, settings, packaging for Windows.

---

## 10. Decisions

- **Team and platform:** solo developer plus Claude. PC only, with Windows as the shipping target.
- **Generator:** an engine-agnostic Rust library of pure functions. It is tested headless and has its own slice viewer.
- **Engine:** custom, in Rust + wgpu, with no Godot. Scale is the main goal, so every gain in performance counts, and the game needs very little of what a general engine provides. See §11.
- **Visuals:** stylized, but gloomy and menacing enough to feel oppressive, with occasional places to rest (see 10.2).
- **Falling:** a fall kills you. You respawn at the last place you were standing safely (see 10.3).

### 10.1 Why a custom engine fits

The game needs very little of what a general engine provides:
- no animated characters
- no rigid-body physics, only a character controller against static geometry
- no level editor, because the world is procedural
- a stylized look that doesn't need Lumen or Nanite

What it does need, a general engine handles badly:
- unbounded coordinates
- GPU-driven rendering of huge, dense, constantly streaming geometry
- occlusion culling of geometry generated at runtime
- a far field composited from several representations

**The cost:** a few weeks of engine bring-up before the first walk-through (Phases 1–2), and owning every bug. To limit both, use proven crates for everything that isn't the core problem (§11.1).

### 10.2 Stylized oppression and places to rest

- **Oppression:**
  - low-key lighting and a narrow, cold palette
  - dense fog with aerial perspective
  - heavy black shapes in the style of Nihei's ink
  - low ceilings over infinite width
  - sounds of machinery
- **Possible stylization passes** (try them in the Phase 2 greybox):
  - posterized or dithered shading
  - edge or ink outlines
  - film grain
- **Places to rest (sanctuaries):** placed at the L2 level, each a few km apart along walkable routes. They have:
  - warm, low light
  - a different sound bed
  - a framed vista
  - small human traces such as a bench, plants, a lamp

  Sanctuaries are where the player exhales, and they should feel earned through contrast with the oppressive stretches.

### 10.3 Death by falling, respawn at last safe ground

- **Safe-ground checkpoint:** record the position, as an integer sector plus a float offset. Update it only when the player:
  - has been grounded for at least ~1 s
  - is on a walkable surface
  - is not near an edge (probe a radius around the feet)
  - is not on a moving lift or tram
- **Death:** triggered by a fall height threshold (for example more than ~15 m), or by falling for a set time into a void with no ground below. Let the fall play out for a moment, fade to black, then respawn. The only penalty is lost progress since the checkpoint.
- **Consequence for generation:** edges must be readable, so railing density and lighting along drops matter. Some deliberately unguarded edges keep tension.

### 10.4 The Web: generator redesign (decided 2026-10-18)

Playing the Phase 5 build showed the world was mostly empty air: deck slabs kilometres apart with sparse boxes on them, and one-door buildings that were dead ends. The user chose a redesign, written up in `docs/proposals/the-web.md`. It replaces the L1 and L2 content described in §2 and in `docs/spec/levels.md`; the specs are rewritten before any code (CHECKLIST Phase W).

- **Built is the default; air is carved.** Strata are dense crusts, 70–90 % built, separated by Betweens: gaps that show how far the structure extends. Breathing space comes at every scale. Big views are rare and earned.
- **An infinite web on every axis.** Random walks at three scales (arteries over L2 cells, routes over slots, rooms in L3). Structures grow on any support (standing, hanging, spanning, sideways and more). Buildings have entrances on many floors and are through-routes.
- **Free orientation.** This reverses the 2026-10-17 axis-aligned decision. Rotations are integer-keyed and drawn from constant tables, and every primitive is clipped to its owning cell, so determinism and the ownership rule hold.
- **Ladders and climbing**, with no extra button.
- **Gravity wells**, in the spirit of Toha Heavy Industries: "down" may point in any direction near a furnace. This answers P5.2.4.
- **Horizon promise in a dense world.** Far-visible things are a fixed set of L0/L1 features. Openings and relief large enough to see from afar are L1 data. Every far landmark is a knot that has a route to it and a hub to find there.

---

## 11. Engine architecture

### 11.1 Stack

| Concern | Choice | Why |
|---|---|---|
| Language | Rust | Shared with the generator; safe threading for streaming |
| Window and input | `winit` | Standard; supports raw mouse motion for first-person look |
| GPU | `wgpu` (Vulkan or DX12 backend on Windows) | Compute shaders, indirect drawing, multi-draw-indirect on native, timestamp queries. Draw-call overhead matters little once rendering is GPU-driven |
| Shaders | WGSL, hot-reloaded | Validated by naga |
| Math | `glam` | Fast SIMD; has f64/i64 vector types for world positions |
| Jobs | `rayon` plus a priority queue (`crossbeam` channels) | Generation and meshing run off the main thread, nearest and in-view first |
| Audio | `kira` | Built-in tweening and reverb for drones, footsteps and reverb sized to the space |
| Debug UI | `egui` + `egui-wgpu` | Sliders for fog, LOD and seeds; generator inspectors |
| Profiling | `tracy-client` (CPU and GPU zones) | Hard per-frame budgets are central to the plan |
| Collision | Custom and analytic | See 11.3 |
| Assets | Kit modules generated in code at first; `gltf` import later if needed | No asset pipeline needed early on |

### 11.2 Workspace layout

```
crates/
  gen/       pure generator: seed + integer coords -> primitives, ports, metadata (no GPU deps)
  engine/    renderer, streaming, camera, input, audio wrappers
  game/      controller, respawn, transport, sanctuaries, the binary
tools/
  slice/     CLI that writes PNG cross-sections and connectivity reports from `gen`
  bots/      headless walkers that path through generated cells (CI)
```

### 11.3 Key design choices

- **World position:** an `i64` sector index plus an `f32` offset in a 1024 m sector. Each frame, chunk transforms are computed **relative to the camera** on the CPU using integer sector differences, so the GPU never sees a large float.
- **Depth:** reversed-Z with an infinite far plane and a 32-bit float depth buffer.
- **The generator outputs primitives** (boxes, ramps, cylinders, stair runs) plus module placements. A mesher turns these into triangles on worker threads.
- **Analytic collision.** The controller queries the same primitives directly: a capsule against boxes, ramps and cylinders in nearby cells. There are no triangle collision meshes, and collision is exact, cheap and consistent with the generator.
- **GPU-driven rendering:**
  - all chunk meshes live in large shared vertex and index buffers, managed by a sub-allocator
  - per-instance data lives in a storage buffer
  - a compute pass performs **frustum culling and Hi-Z occlusion culling**, then writes indirect draw arguments, rendered with `multi_draw_indirect`
  - in a dense megastructure, occlusion culling is the single largest performance win
- **LOD bands:**
  - near chunks: full modules
  - mid range: merged L2 meshes
  - far: instanced L1 megaforms
  - horizon: a full-screen SDF raymarch of L0/L1 rules that writes depth
  - transitions between bands use blue-noise dithering
- **Lighting:**
  - forward+ with clustered lights, to handle many small emissive and point lights
  - few shadow-casting lights, by stylistic choice
  - because the world is made of primitives, ambient occlusion and soft shadows can be computed analytically or with SDFs from a GPU buffer of nearby primitives
  - **Decision, ambient occlusion (P5.1.4, 2026-10-18): analytic box-distance AO.** Both candidates were built (`engine::ao`) and compared on a corner room and on the densest sampled L3 block at 1440p. SSAO costs 0.39 ms but is nearly invisible at a radius that does not halo, and grainy without a blur pass. The analytic one measures the distance to the nearest of up to 256 boxes along the normal, culled per 8×8 tile: 0.47 ms, noise-free, stable under motion, and exact for a world of boxes. SSAO stays selectable as a fallback for non-box shapes (cylinders). The analytic mode needs a buffer of nearby boxes, so streaming L3 blocks into the game must also feed `Renderer::set_ao_boxes`.
- **Fog:** froxel volumetric fog, a compute pass over a 3D grid fitted to the camera's view.
- **Post-processing:** tone mapping, then the stylization pass (posterize, dither, ink outlines from depth and normals, grain), then FXAA or SMAA.

### 11.4 Frame budget (60 fps = 16.6 ms on the target GPU)

| Pass | Budget |
|---|---|
| Culling (compute, including Hi-Z) | ≤ 0.5 ms |
| Geometry (depth prepass and shading) | ≤ 6 ms |
| Volumetric fog | ≤ 2 ms |
| SDF horizon | ≤ 2 ms |
| Post-processing and stylization | ≤ 1.5 ms |
| Headroom | ~4.5 ms |

CPU main thread: at most 4 ms per frame. Generation and meshing run on worker threads. Uploads are capped at a few MB per frame through a staging belt.

#### Generation cost (P0.11.4)

`cargo bench -p gen --bench cells`, single thread, release build, recorded 2026-10-17 at `GEN_VERSION` 3 on **native Windows** (x86_64-pc-windows-gnu toolchain 1.97.1, because this machine has no MSVC linker). The same bench on WSL2 gives the same numbers within noise.

| Measurement | Windows | Budget |
|---|---|---|
| L1 cell, mean over 32 cells that emit geometry | 24 µs | 5 ms |
| L2 cell, mean over 2048 children of 4 L1 parents (most are empty air), L1 parent cached | 94 µs | 2 ms |
| L2 cell, busy cells only (83 of them: decks, skywalks, towers, masses, railings), L1 parent cached | 1.44 ms | 2 ms |
| L2 cell, first 8 busy cells, with their L1 parent generated per cell | 0.85 ms | 2 ms |

**Budget status:** all inside budget, but the busy L2 cell is close. An earlier WSL2 run at `GEN_VERSION` 1 measured 97 µs for busy cells (34 of them, before towers, trunk-attached masses and the 2 m railing probe), so the busy cost grew about 15× while the cell set also changed. The railing pass probes every walkable edge in 2 m cells against every primitive and is the likely cost; follow-up P0.11.4c. The numbers include ports, skywalks, lift shafts, towers, sanctuary, vista, masses and railings. Connectivity validation (`check_cell`) is not part of generation.

### 11.5 Development environment caveat

Development happens in WSL2. A wgpu window there may fall back to software rendering or behave differently from native Windows. Plan to:
- run and profile the game natively on Windows (`cargo` on the Windows side, or cross-compile to `x86_64-pc-windows-msvc`/`-gnu`)
- keep WSL for the `gen` crate, tests, the slice tool, bots and headless render tests (offscreen wgpu with lavapipe: correctness only, not performance)

---

## Sources

- Games: [Sector №9](https://industry-tamers.itch.io/sector-9) · [Megastructure (Pierral46)](https://pierral46.itch.io/megastructure) · [80.lv – Mik](https://80.lv/articles/explore-a-desolate-megastructure-in-this-upcoming-unity-powered-game) · [Killscreen – BLAME!'s influence](https://www.killscreen.com/influence-blame-videogame-architecture-rising/) · [NaissanceE (Wikipedia)](https://en.wikipedia.org/wiki/NaissanceE) · [project_Structure_01](https://store.steampowered.com/app/3156820/project_Structure_01/) · [Megastructure Games curator](https://store.steampowered.com/curator/45572866-Megastructure-Games/) · [Megastructure (Scatter)](https://store.steampowered.com/app/3967000/Megastructure/) · [BLACKSHARD](https://80.lv/articles/walk-through-a-megastructure-in-this-first-person-exploration-game-with-naissancee-vibes) · [Blame! (Wikipedia)](https://en.wikipedia.org/wiki/Blame!)
- Generation: [marian42 WFC article](https://marian42.de/article/wfc/) · [marian42 itch](https://marian42.itch.io/wfc) · [Nested WFC (arXiv 2308.07307)](https://arxiv.org/pdf/2308.07307) · [Hierarchical WFC (AIIDE)](https://ojs.aaai.org/index.php/AIIDE/article/view/27498) · [Hierarchical Semantic WFC](https://research.tudelft.nl/en/publications/hierarchical-semantic-wave-function-collapse) · [Model synthesis](https://en.wikipedia.org/wiki/Model_synthesis) · [No Man's Sky generation](https://www.thesixthaxis.com/?p=223047) · [Townscaper grid](https://boristhebrave.com/docs/sylves/1/articles/tutorials/townscaper.html)
- Precision and engines: [Godot large world coordinates](https://docs.godotengine.org/es/4.x/tutorials/physics/large_world_coordinates.html) · [big_space](https://github.com/aevyrie/big_space) · [UE 5.7 PCG](https://creativebloq.com/3d/video-game-design/the-unreal-engine-5-7-procedural-content-generation-update-explained) · [Infinite world in UE5 thesis](https://dcgi.fel.cvut.cz/en/theses/2025/juzaluk2/) · [Bevy 0.14 meshlets](https://bevy.org/news/bevy-0-14/) · [Godot 4.6](https://gamefromscratch.com/godot-4-6-released/) · [three.js WebGPU guide](https://www.utsubo.com/blog/webgpu-threejs-migration-guide)
- Rendering and scale: [IQ – SDF repetition](https://iquilezles.org/articles/sdfrepetition) · [Manifold Garden rendering](https://blog.siggraph.org/2020/12/manifold-garden-takes-control-with-a-gpu-rendering-pipeline.html/) · [SotC visual design](https://www.gamedeveloper.com/design/the-art-of-shadow-of-the-colossus-4-6-visual-design) · [Octahedral impostors (Unity)](https://docs.unity3d.com/Packages/com.unity.industry.toolkit@3.2/manual/lod-generator.html)
