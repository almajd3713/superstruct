// WGSL port of the L1 megaform rules that decide trunks and decks (`crates/gen/src/l1.rs`,
// `docs/spec/levels.md` §4), for the SDF horizon. It is a port, not a re-derivation: the hash,
// RNG, draw order and integer geometry follow the Rust bit for bit.
//
// Not ported (rare, or too thin to matter at horizon range): giant-cavity cells, the rim bites
// they cause, bridges, and the 2^-59 rejection loop of `Rng::below`.
//
// Standalone, prepended with the generated tables (`horizon::prelude`). 64-bit values are
// `vec2<u32>` as (low, high); signed 64-bit cell coordinates are the two's-complement bits.

alias U64 = vec2<u32>;

const GAMMA: U64 = vec2<u32>(0x7F4A7C15u, 0x9E3779B9u);
const MIX1: U64 = vec2<u32>(0x1CE4E5B9u, 0xBF58476Du);
const MIX2: U64 = vec2<u32>(0x133111EBu, 0x94D049BBu);
const ZERO64: U64 = vec2<u32>(0u, 0u);

const CELL: i32 = 65536;      // L1 cell edge in quanta (16 per metre)
const SEP: i32 = 1024;        // separation between footprints and from the faces
const SALT_REGIME: u32 = 0x0101u;
const SALT_RARE_STAIR: u32 = 0x0104u;
const SALT_TRUNK: u32 = 0x0201u;
const SALT_DECK: u32 = 0x0202u;
const SALT_SHAFT: u32 = 0x0203u;
const SALT_CHASM: u32 = 0x0204u;
const SALT_CORE: u32 = 0x0207u;

// ---- 64-bit arithmetic ------------------------------------------------------------------

// Full 64-bit product of two u32, as (low, high).
fn mul32_wide(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xffffu;
    let a1 = a >> 16u;
    let b0 = b & 0xffffu;
    let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);
    return vec2<u32>(
        (p00 & 0xffffu) | (mid << 16u),
        p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u),
    );
}

fn add64(a: U64, b: U64) -> U64 {
    let lo = a.x + b.x;
    return vec2<u32>(lo, a.y + b.y + select(0u, 1u, lo < a.x));
}

// Low 64 bits of the product.
fn mul64(a: U64, b: U64) -> U64 {
    let w = mul32_wide(a.x, b.x);
    return vec2<u32>(w.x, w.y + a.x * b.y + a.y * b.x);
}

// Logical shift right, 0 < n < 32.
fn shr64(a: U64, n: u32) -> U64 {
    return vec2<u32>((a.x >> n) | (a.y << (32u - n)), a.y >> n);
}

// Arithmetic shift right of a signed 64-bit value, 0 < n < 32: floor division by 2^n.
fn i64_sar(a: U64, n: u32) -> U64 {
    return vec2<u32>((a.x >> n) | (a.y << (32u - n)), bitcast<u32>(bitcast<i32>(a.y) >> n));
}

fn i64_from_i32(v: i32) -> U64 {
    return vec2<u32>(bitcast<u32>(v), select(0u, 0xffffffffu, v < 0));
}

fn i64_add_i32(a: U64, d: i32) -> U64 {
    return add64(a, i64_from_i32(d));
}

// ---- hash and RNG (`docs/spec/hashing.md`) -----------------------------------------------

fn mix64(z0: U64) -> U64 {
    var z = z0;
    z = mul64(z ^ shr64(z, 30u), MIX1);
    z = mul64(z ^ shr64(z, 27u), MIX2);
    return z ^ shr64(z, 31u);
}

fn absorb(h: U64, v: U64) -> U64 {
    return mix64(add64(h ^ v, GAMMA));
}

fn hash64(seed: U64, level: u32, x: U64, y: U64, z: U64, salt: u32) -> U64 {
    var h = mix64(add64(seed, GAMMA));
    h = absorb(h, vec2<u32>(level, 0u));
    h = absorb(h, x);
    h = absorb(h, y);
    h = absorb(h, z);
    return absorb(h, vec2<u32>(salt, 0u));
}

fn rng_next(s: ptr<function, U64>) -> U64 {
    *s = add64(*s, GAMMA);
    return mix64(*s);
}

// Uniform in [0, n) by Lemire's multiply-shift; n < 2^32. The rejection branch of the Rust
// version fires with probability below n / 2^64 and is not ported.
fn rng_below(s: ptr<function, U64>, n: u32) -> u32 {
    let x = rng_next(s);
    let a = mul32_wide(x.x, n);
    let b = mul32_wide(x.y, n);
    let w1 = a.y + b.x;
    return b.y + select(0u, 1u, w1 < a.y);
}

fn rng_range(s: ptr<function, U64>, lo: i32, hi: i32) -> i32 {
    return lo + i32(rng_below(s, u32(hi - lo)));
}

// `next_f32() < p` for a threshold `t = ceil(p * 2^24)`: the 24 random bits compared as integers.
fn rng_chance(s: ptr<function, U64>, t: u32) -> bool {
    return (rng_next(s).y >> 8u) < t;
}

// Index of the weight bucket drawn, 0 when every weight is 0 (and then no draw happens).
fn rng_pick4(s: ptr<function, U64>, w: vec4<u32>) -> u32 {
    let total = w.x + w.y + w.z + w.w;
    if (total == 0u) {
        return 0u;
    }
    let r = rng_below(s, total);
    if (r < w.x) { return 0u; }
    if (r < w.x + w.y) { return 1u; }
    if (r < w.x + w.y + w.z) { return 2u; }
    return 3u;
}

// ---- regime ------------------------------------------------------------------------------

// (density, verticality) of an L0 cell.
fn regime_dv(seed: U64, x: U64, y: U64, z: U64) -> vec2<u32> {
    var rng = hash64(seed, 0u, x, y, z, SALT_REGIME);
    var total = 0u;
    for (var i = 0; i < 5; i = i + 1) {
        total = total + STYLE_W[i];
    }
    let r = rng_below(&rng, total);
    var style = 4;
    var acc = 0u;
    for (var i = 0; i < 5; i = i + 1) {
        acc = acc + STYLE_W[i];
        if (r < acc) {
            style = i;
            break;
        }
    }
    let d = STYLE_D[style];
    let v = STYLE_V[style];
    let density = rng_range(&rng, d.x, d.y + 1);
    let verticality = rng_range(&rng, v.x, v.y + 1);
    return vec2<u32>(u32(density), u32(verticality));
}

// ---- geometry helpers (quanta, cell-local) -----------------------------------------------

struct Rect {
    x0: i32,
    z0: i32,
    x1: i32,
    z1: i32,
};

fn square(x0: i32, z0: i32, side: i32) -> Rect {
    return Rect(x0, z0, x0 + side, z0 + side);
}

// True when the squared distance from the point to the rectangle is at least need^2. Written to
// stay inside i32: either axis alone reaching `need` settles it.
fn point_far(r: Rect, x: i32, z: i32, need: i32) -> bool {
    let dx = max(max(r.x0 - x, x - r.x1), 0);
    let dz = max(max(r.z0 - z, z - r.z1), 0);
    return dx >= need || dz >= need || dx * dx + dz * dz >= need * need;
}

fn rect_far(a: Rect, b: Rect, need: i32) -> bool {
    let dx = max(max(b.x0 - a.x1, a.x0 - b.x1), 0);
    let dz = max(max(b.z0 - a.z1, a.z0 - b.z1), 0);
    return dx >= need || dz >= need || dx * dx + dz * dz >= need * need;
}

// `lo + step * range(0, (hi - lo) / step + 1)`.
fn step_range(s: ptr<function, U64>, lo: i32, hi: i32, step: i32) -> i32 {
    return lo + step * rng_range(s, 0, (hi - lo) / step + 1);
}

struct OptRect {
    ok: bool,
    r: Rect,
};

const NO_RECT: OptRect = OptRect(false, Rect(0, 0, 0, 0));

// ---- L1 features -------------------------------------------------------------------------

// Lift core footprint of an L1 column.
fn core_footprint(seed: U64, cx: U64, cz: U64) -> Rect {
    var rng = hash64(seed, 1u, cx, ZERO64, cz, SALT_CORE);
    let side = rng_range(&rng, 4, 9) * 64;
    let x0 = step_range(&rng, SEP, CELL - SEP - side, 64);
    let z0 = step_range(&rng, SEP, CELL - SEP - side, 64);
    return square(x0, z0, side);
}

// Infinite-staircase footprint of an L1 column, if it has one.
fn stair_footprint(seed: U64, cx: U64, cz: U64) -> OptRect {
    let lx0 = i64_sar(cx, 3u);
    let lz0 = i64_sar(cz, 3u);
    let start = hash64(seed, 0u, lx0, ZERO64, lz0, SALT_RARE_STAIR);
    var rng = start;
    if (rng_below(&rng, 1000u) >= RARE_STAIR_PER_MILLE) {
        return NO_RECT;
    }
    let lx = rng_below(&rng, 8u);
    let lz = rng_below(&rng, 8u);
    if (lx != (cx.x & 7u) || lz != (cz.x & 7u)) {
        return NO_RECT;
    }
    // Continue the staircase's own stream after presence and L1 column.
    var r2 = start;
    rng_below(&r2, 1000u);
    rng_below(&r2, 8u);
    rng_below(&r2, 8u);
    let hi = CELL - SEP - 1024;
    let x0 = step_range(&r2, SEP, hi, 256);
    let z0 = step_range(&r2, SEP, hi, 256);
    let rect = square(x0, z0, 1024);
    if (rect_far(rect, core_footprint(seed, cx, cz), SEP)) {
        return OptRect(true, rect);
    }
    return NO_RECT;
}

struct Trunk {
    ok: bool,
    cx: i32,
    cz: i32,
    r: i32,
    a: i32,
    b: i32,
};

// The trunk of a column span (x, sy, z).
fn l1_trunk(seed: U64, sx: U64, sy: U64, sz: U64) -> Trunk {
    var t = Trunk(false, 0, 0, 0, 0, 0);
    let dv = regime_dv(seed, i64_sar(sx, 3u), sy, i64_sar(sz, 3u));
    var rng = hash64(seed, 1u, sx, sy, sz, SALT_TRUNK);
    if (!rng_chance(&rng, TRUNK_T[dv.y])) {
        return t;
    }
    let drawn = rng_range(&rng, 6, 26) * 256;
    let lo = drawn + SEP;
    let cx = step_range(&rng, lo, CELL - lo, 256);
    let cz = step_range(&rng, lo, CELL - lo, 256);
    var a = 0;
    var b = 7;
    if (!rng_chance(&rng, 8388608u)) {
        a = rng_range(&rng, 0, 8);
        b = rng_range(&rng, a, 8);
    }
    let core = core_footprint(seed, sx, sz);
    let stair = stair_footprint(seed, sx, sz);
    // Shrink from the drawn radius down to 96 m in 16 m steps until clear of the earlier ones.
    for (var r = drawn; r >= 1536; r = r - 256) {
        let need = r + SEP;
        if (point_far(core, cx, cz, need) && (!stair.ok || point_far(stair.r, cx, cz, need))) {
            return Trunk(true, cx, cz, r, a, b);
        }
    }
    return t;
}

fn place_rect(s: ptr<function, U64>, w: i32, d: i32, step: i32) -> Rect {
    let x0 = step_range(s, SEP, CELL - SEP - w, step);
    let z0 = step_range(s, SEP, CELL - SEP - d, step);
    return Rect(x0, z0, x0 + w, z0 + d);
}

// Keeps clear of the core, the staircase and the trunk of the same column span.
fn clear_of_column(seed: U64, sx: U64, sy: U64, sz: U64, r: Rect) -> bool {
    if (!rect_far(r, core_footprint(seed, sx, sz), SEP)) {
        return false;
    }
    let stair = stair_footprint(seed, sx, sz);
    if (stair.ok && !rect_far(r, stair.r, SEP)) {
        return false;
    }
    let t = l1_trunk(seed, sx, sy, sz);
    if (t.ok) {
        if (!point_far(r, t.cx, t.cz, t.r + SEP)) {
            return false;
        }
    }
    return true;
}

fn l1_shaft(seed: U64, sx: U64, sy: U64, sz: U64) -> OptRect {
    let dv = regime_dv(seed, i64_sar(sx, 3u), sy, i64_sar(sz, 3u));
    var rng = hash64(seed, 1u, sx, sy, sz, SALT_SHAFT);
    if (!rng_chance(&rng, SHAFT_T[dv.x])) {
        return NO_RECT;
    }
    let w = rng_range(&rng, 4, 33) * 256;
    let depth = rng_range(&rng, 4, 33) * 256;
    let r = place_rect(&rng, w, depth, 256);
    if (clear_of_column(seed, sx, sy, sz, r)) {
        return OptRect(true, r);
    }
    return NO_RECT;
}

// The chasm of L1 cell (cx, cy, cz).
fn l1_chasm(seed: U64, cx: U64, cy: U64, cz: U64) -> OptRect {
    let sy = i64_sar(cy, 3u);
    let dv = regime_dv(seed, i64_sar(cx, 3u), sy, i64_sar(cz, 3u));
    var rng = hash64(seed, 1u, cx, cy, cz, SALT_CHASM);
    if (!rng_chance(&rng, CHASM_T[dv.x])) {
        return NO_RECT;
    }
    let w = rng_range(&rng, 4, 33) * 1024;
    let depth = rng_range(&rng, 4, 33) * 1024;
    let r = place_rect(&rng, w, depth, 1024);
    if (!clear_of_column(seed, cx, sy, cz, r)) {
        return NO_RECT;
    }
    let shaft = l1_shaft(seed, cx, sy, cz);
    if (shaft.ok && !rect_far(r, shaft.r, SEP)) {
        return NO_RECT;
    }
    return OptRect(true, r);
}

// ---- one cell ----------------------------------------------------------------------------

struct CellData {
    trunk_ok: bool,
    t_cx: i32,
    t_cz: i32,
    t_r: i32,
    deck_n: u32,
    top: array<i32, 3>,
    thick: array<i32, 3>,
    core: Rect,
    stair: OptRect,
    shaft: OptRect,
    chasm: OptRect,
};

// Everything the horizon needs from L1 cell (cx, cy, cz): its trunk piece, decks and holes.
fn l1_cell_data(seed: U64, cx: U64, cy: U64, cz: U64) -> CellData {
    var d: CellData;
    let sy = i64_sar(cy, 3u);
    let t = l1_trunk(seed, cx, sy, cz);
    let j = i32(cy.x & 7u);
    d.trunk_ok = t.ok && j >= t.a && j <= t.b;
    d.t_cx = t.cx;
    d.t_cz = t.cz;
    d.t_r = t.r;

    // Deck layer: a 4x4 region of cells shares one draw.
    let rx = i64_sar(cx, 2u);
    let rz = i64_sar(cz, 2u);
    let dv = regime_dv(seed, i64_sar(rx, 1u), i64_sar(cy, 3u), i64_sar(rz, 1u));
    var rng = hash64(seed, 1u, rx, cy, rz, SALT_DECK);
    let base = dv.x * 4u;
    let n = rng_pick4(&rng, vec4<u32>(DECK_W[base], DECK_W[base + 1u], DECK_W[base + 2u], DECK_W[base + 3u]));
    d.deck_n = n;
    let step = 64;
    for (var i = 0; i < 3; i = i + 1) {
        if (u32(i) >= n) {
            break;
        }
        let ni = i32(n);
        let centre = (2 * i + 1) * CELL / (2 * ni) / step * step;
        let jitter = CELL / (4 * ni) / step;
        d.top[i] = centre + step * rng_range(&rng, -jitter, jitter + 1);
        d.thick[i] = rng_range(&rng, 2, 13) * step;
    }
    d.core = core_footprint(seed, cx, cz);
    d.stair = stair_footprint(seed, cx, cz);
    d.shaft = l1_shaft(seed, cx, sy, cz);
    d.chasm = l1_chasm(seed, cx, cy, cz);
    return d;
}

fn in_rect_m(r: Rect, x: f32, z: f32) -> bool {
    return f32(r.x0) / 16.0 <= x && x < f32(r.x1) / 16.0 && f32(r.z0) / 16.0 <= z && z < f32(r.z1) / 16.0;
}

// 0 void, 1 trunk, 2 deck, for a point `p` in metres inside the cell.
fn l1_classify(d: CellData, p: vec3<f32>) -> u32 {
    if (d.trunk_ok) {
        let r = f32(d.t_r) / 16.0;
        let dx = p.x - f32(d.t_cx) / 16.0;
        let dz = p.z - f32(d.t_cz) / 16.0;
        if (p.y >= 0.0 && p.y < 4096.0 && dx * dx + dz * dz < r * r) {
            return 1u;
        }
    }
    if (p.x < 0.0 || p.x >= 4096.0 || p.z < 0.0 || p.z >= 4096.0) {
        return 0u;
    }
    if (in_rect_m(d.core, p.x, p.z)
        || (d.stair.ok && in_rect_m(d.stair.r, p.x, p.z))
        || (d.shaft.ok && in_rect_m(d.shaft.r, p.x, p.z))
        || (d.chasm.ok && in_rect_m(d.chasm.r, p.x, p.z))) {
        return 0u;
    }
    for (var i = 0; i < 3; i = i + 1) {
        if (u32(i) >= d.deck_n) {
            break;
        }
        if (f32(d.top[i] - d.thick[i]) / 16.0 <= p.y && p.y < f32(d.top[i]) / 16.0) {
            return 2u;
        }
    }
    return 0u;
}

// ---- Oriented pieces (primitives.md §9, TEC-07) ---------------------------------------------
// The SDF of an oriented box is the axis-aligned SDF in its frame, cut by the cell clip with
// `max` (a lower bound). `rot` is the frame as a unit quaternion (x, y, z, w); the box is the
// frame-local box `centre ± half`. Checked against `worldgen::orient::Oriented::sdf` in
// `tests/horizon.rs`.

fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

fn sdf_aabb(p: vec3<f32>, centre: vec3<f32>, half_size: vec3<f32>) -> f32 {
    let d = abs(p - centre) - half_size;
    return length(max(d, vec3<f32>(0.0))) + min(max(d.x, max(d.y, d.z)), 0.0);
}

fn sdf_oriented_box(
    p: vec3<f32>, origin: vec3<f32>, rot: vec4<f32>, centre: vec3<f32>, half_size: vec3<f32>,
    clip_min: vec3<f32>, clip_max: vec3<f32>,
) -> f32 {
    let local = quat_rotate(vec4<f32>(-rot.xyz, rot.w), p - origin);
    let shape = sdf_aabb(local, centre, half_size);
    let clip = sdf_aabb(p, (clip_min + clip_max) * 0.5, (clip_max - clip_min) * 0.5);
    return max(shape, clip);
}
