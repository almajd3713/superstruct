// Fills the window of L1 cell records the horizon pass marches through. Prepended with the
// generated tables and `horizon.wgsl`.
//
// A record is 32 words:
//   0      flags: bit 0 trunk piece, bits 1-2 deck count, bit 3 staircase, bit 4 shaft, bit 5 chasm
//   1..3   trunk centre x, centre z, radius (quanta)
//   4..9   (top, thickness) of up to three decks (quanta)
//   10..13 core rectangle x0 z0 x1 z1
//   14..17 staircase rectangle, 18..21 shaft rectangle, 22..25 chasm rectangle
//   26..31 unused

struct CacheParams {
    seed: vec2<u32>,
    // Cell coordinates of the window's minimum corner.
    origin_x: vec2<u32>,
    origin_y: vec2<u32>,
    origin_z: vec2<u32>,
    n: u32,
};

@group(0) @binding(0) var<uniform> params: CacheParams;
@group(0) @binding(1) var<storage, read_write> records: array<u32>;

fn put_rect(base: u32, r: Rect) {
    records[base] = bitcast<u32>(r.x0);
    records[base + 1u] = bitcast<u32>(r.z0);
    records[base + 2u] = bitcast<u32>(r.x1);
    records[base + 3u] = bitcast<u32>(r.z1);
}

@compute @workgroup_size(64)
fn cs_cache(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = params.n;
    let i = id.x;
    if (i >= n * n * n) {
        return;
    }
    let ix = i % n;
    let iy = (i / n) % n;
    let iz = i / (n * n);
    let d = l1_cell_data(
        params.seed,
        i64_add_i32(params.origin_x, i32(ix)),
        i64_add_i32(params.origin_y, i32(iy)),
        i64_add_i32(params.origin_z, i32(iz)),
    );
    let b = i * 32u;
    var flags = select(0u, 1u, d.trunk_ok) | (d.deck_n << 1u);
    flags = flags | select(0u, 8u, d.stair.ok) | select(0u, 16u, d.shaft.ok) | select(0u, 32u, d.chasm.ok);
    records[b] = flags;
    records[b + 1u] = bitcast<u32>(d.t_cx);
    records[b + 2u] = bitcast<u32>(d.t_cz);
    records[b + 3u] = bitcast<u32>(d.t_r);
    for (var k = 0; k < 3; k = k + 1) {
        records[b + 4u + u32(k) * 2u] = bitcast<u32>(d.top[k]);
        records[b + 5u + u32(k) * 2u] = bitcast<u32>(d.thick[k]);
    }
    put_rect(b + 10u, d.core);
    put_rect(b + 14u, d.stair.r);
    put_rect(b + 18u, d.shaft.r);
    put_rect(b + 22u, d.chasm.r);
}
