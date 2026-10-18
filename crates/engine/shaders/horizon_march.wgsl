// The SDF horizon: a full-screen pass that walks each pixel's ray through the L1 cell records
// (`horizon_cache.wgsl`) beyond the far band and draws the first trunk or deck it meets, with
// the scene's own lighting and fog, writing depth. `common.wgsl` and `shade.wgsl` come first.

struct HorizonParams {
    inv_view_proj: mat4x4<f32>,
    // Camera position inside its L1 cell, metres.
    cam_local: vec3<f32>,
    // Where marching starts and stops, metres from the camera.
    t_start: f32,
    t_end: f32,
    // The window holds n^3 cells and the camera is in cell (half, half, half).
    n: u32,
    half: u32,
    pad: u32,
    screen: vec2<f32>,
};

@group(1) @binding(0) var<uniform> hz: HorizonParams;
@group(1) @binding(1) var<storage, read> records: array<u32>;

const CELL_M: f32 = 4096.0;
const MAX_STEPS: i32 = 256;

struct Hit {
    t: f32,
    n: vec3<f32>,
};

const NO_HIT = Hit(1.0e30, vec3<f32>(0.0));

fn rec_i(base: u32, k: u32) -> i32 {
    return bitcast<i32>(records[base + k]);
}

fn rec_rect(base: u32, k: u32) -> vec4<f32> {
    // x0 z0 x1 z1 in metres, cell-local.
    return vec4<f32>(
        f32(rec_i(base, k)),
        f32(rec_i(base, k + 1u)),
        f32(rec_i(base, k + 2u)),
        f32(rec_i(base, k + 3u)),
    ) / 16.0;
}

// First time at or after `t` at which the ray is outside the rectangle `r` (x0 z0 x1 z1, world
// metres), assuming it is inside at `t`; also which axis it leaves through.
fn rect_exit(r: vec4<f32>, o: vec3<f32>, d: vec3<f32>) -> vec2<f32> {
    var tx = 1.0e30;
    if (d.x > 1.0e-9) { tx = (r.z - o.x) / d.x; } else if (d.x < -1.0e-9) { tx = (r.x - o.x) / d.x; }
    var tz = 1.0e30;
    if (d.z > 1.0e-9) { tz = (r.w - o.z) / d.z; } else if (d.z < -1.0e-9) { tz = (r.y - o.z) / d.z; }
    return select(vec2<f32>(tz, 1.0), vec2<f32>(tx, 0.0), tx <= tz);
}

fn inside_rect(r: vec4<f32>, x: f32, z: f32) -> bool {
    return r.x <= x && x < r.z && r.y <= z && z < r.w;
}

// Nearest surface of one cell's trunk and decks along the ray within [tin, tout]. `origin` is
// the cell's minimum corner in window metres and `entry` the normal of the face the ray came in by.
fn cell_hit(base: u32, origin: vec3<f32>, o: vec3<f32>, d: vec3<f32>, tin: f32, tout: f32, entry: vec3<f32>) -> Hit {
    var best = NO_HIT;
    let flags = records[base];

    if ((flags & 1u) != 0u) {
        let c = origin.xz + vec2<f32>(f32(rec_i(base, 1u)), f32(rec_i(base, 2u))) / 16.0;
        let r = f32(rec_i(base, 3u)) / 16.0;
        let a = dot(d.xz, d.xz);
        let oc = o.xz - c;
        if (a > 1.0e-12) {
            let b = dot(oc, d.xz);
            let disc = b * b - a * (dot(oc, oc) - r * r);
            if (disc >= 0.0) {
                let s = sqrt(disc);
                let t0 = (-b - s) / a;
                let t1 = (-b + s) / a;
                if (t1 > tin && t0 < tout) {
                    let t = max(t0, tin);
                    let y = o.y + d.y * t;
                    if (y >= origin.y && y < origin.y + CELL_M) {
                        // Entering through the cell face (inside the cylinder) or through its side.
                        var n = entry;
                        if (t0 >= tin) {
                            let p = o.xz + d.xz * t - c;
                            n = vec3<f32>(p.x, 0.0, p.y) / r;
                        }
                        best = Hit(t, n);
                    }
                }
            }
        } else if (dot(oc, oc) < r * r) {
            best = Hit(tin, entry);
        }
    }

    let n_decks = (flags >> 1u) & 3u;
    for (var i = 0u; i < 3u; i = i + 1u) {
        if (i >= n_decks) {
            break;
        }
        let top = f32(rec_i(base, 4u + i * 2u)) / 16.0;
        let thick = f32(rec_i(base, 5u + i * 2u)) / 16.0;
        let y1 = origin.y + top;
        let y0 = y1 - thick;
        var lo = tin;
        var hi = tout;
        var enter = entry;
        if (abs(d.y) < 1.0e-7) {
            if (o.y < y0 || o.y >= y1) {
                continue;
            }
        } else {
            let ta = (y0 - o.y) / d.y;
            let tb = (y1 - o.y) / d.y;
            let t_lo = min(ta, tb);
            let t_hi = max(ta, tb);
            if (t_lo > lo) {
                lo = t_lo;
                enter = vec3<f32>(0.0, select(-1.0, 1.0, d.y < 0.0), 0.0);
            }
            hi = min(hi, t_hi);
        }
        if (lo >= hi || lo >= best.t) {
            continue;
        }
        // Step out of any holes (core, staircase, shaft, chasm) the ray starts inside.
        var t = lo;
        var normal = enter;
        for (var pass_ = 0; pass_ < 5; pass_ = pass_ + 1) {
            var moved = false;
            for (var h = 0u; h < 4u; h = h + 1u) {
                if (h >= 1u && (flags & (4u << h)) == 0u) {
                    continue;
                }
                let r = rec_rect(base, 10u + h * 4u) + vec4<f32>(origin.x, origin.z, origin.x, origin.z);
                let p = o + d * t;
                if (inside_rect(r, p.x, p.z)) {
                    let e = rect_exit(r, o, d);
                    t = e.x + 1.0e-3;
                    normal = select(vec3<f32>(0.0, 0.0, -sign(d.z)), vec3<f32>(-sign(d.x), 0.0, 0.0), e.y < 0.5);
                    moved = true;
                }
            }
            if (!moved) {
                break;
            }
        }
        if (t < hi && t < best.t) {
            best = Hit(t, normal);
        }
    }
    return best;
}

fn march(o: vec3<f32>, d_in: vec3<f32>) -> Hit {
    let d = select(d_in, vec3<f32>(1.0e-9), abs(d_in) < vec3<f32>(1.0e-9));
    let step = vec3<i32>(sign(d));
    let inv = 1.0 / d;
    var t = hz.t_start;
    var cell = vec3<i32>(floor((o + d * t) / CELL_M));
    var tmax = (vec3<f32>(cell + max(step, vec3<i32>(0))) * CELL_M - o) * inv;
    let tdelta = abs(CELL_M * inv);
    let n = i32(hz.n);
    var entry = -vec3<f32>(step) * vec3<f32>(abs(d) >= max(abs(d.yzx), abs(d.zxy)));
    for (var i = 0; i < MAX_STEPS; i = i + 1) {
        if (any(cell < vec3<i32>(0)) || any(cell >= vec3<i32>(n)) || t > hz.t_end) {
            break;
        }
        let tout = min(min(tmax.x, tmax.y), tmax.z);
        let idx = u32((cell.z * n + cell.y) * n + cell.x);
        let hit = cell_hit(idx * 32u, vec3<f32>(cell) * CELL_M, o, d, t, tout, entry);
        if (hit.t < 1.0e29) {
            return hit;
        }
        t = tout;
        if (tmax.x <= tmax.y && tmax.x <= tmax.z) {
            cell.x = cell.x + step.x;
            tmax.x = tmax.x + tdelta.x;
            entry = vec3<f32>(-f32(step.x), 0.0, 0.0);
        } else if (tmax.y <= tmax.z) {
            cell.y = cell.y + step.y;
            tmax.y = tmax.y + tdelta.y;
            entry = vec3<f32>(0.0, -f32(step.y), 0.0);
        } else {
            cell.z = cell.z + step.z;
            tmax.z = tmax.z + tdelta.z;
            entry = vec3<f32>(0.0, 0.0, -f32(step.z));
        }
    }
    return NO_HIT;
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

struct FsOut {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_march(@builtin(position) frag: vec4<f32>) -> FsOut {
    // Pixel to a ray: unproject a point on the near plane and aim from the camera through it.
    let ndc = vec2<f32>(frag.x / hz.screen.x * 2.0 - 1.0, 1.0 - frag.y / hz.screen.y * 2.0);
    let pn = hz.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(pn.xyz / pn.w);
    let o = vec3<f32>(f32(hz.half) * CELL_M) + hz.cam_local;
    let hit = march(o, dir);
    if (hit.t >= 1.0e29) {
        discard;
    }
    let rel = dir * hit.t;
    var colour = lit_colour(normalize(hit.n), 0u);
    colour = apply_fog(colour, rel);
    let clip = globals.view_proj * vec4<f32>(rel, 1.0);
    var out: FsOut;
    out.colour = vec4<f32>(colour, 1.0);
    out.depth = clip.z / clip.w;
    return out;
}
