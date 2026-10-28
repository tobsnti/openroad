use bevy::asset::RenderAssetUsages;
use bevy::math::Vec3;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::Mesh;
use std::collections::HashMap;

use crate::assets::m::{TerrainBlock, JMXVMAPM};
use crate::util::mesh::reverse_winding_u32;

/// Canonical block winding: correct under a *non-mirrored* (positive-determinant)
/// placement, matching the convention raw `.bms` data uses. `BLOCK_INDICES` is
/// authored for the mirrored (negative-determinant) terrain placement, i.e. it is
/// already the reversed form — so the canonical winding is its per-triangle
/// reverse. The shared winding rule (`needs_winding_reversal`) then reverses this
/// again for mirrored placements, reproducing `BLOCK_INDICES` exactly.
pub fn canonical_block_indices() -> Vec<u32> {
    let mut indices = BLOCK_INDICES.to_vec();
    reverse_winding_u32(&mut indices);
    indices
}

/// The two triangles of one terrain quad, in the mesh's winding convention,
/// split along the diagonal the parity of the quad's grid indices selects.
///
/// Idea: a quad's four corner heights are almost never coplanar, so the split
/// diagonal *is* the quad's surface — and the mover walks on whichever diagonal
/// [`height_in_quad`](crate::assets::nvm::height_map) picks, which is the one
/// the original's `FindHeight` picks: `(cell_x % 2) == (cell_z % 2)`. A mesh
/// that always split h00-h11 therefore drew ground the mover was not standing
/// on wherever the parities differ — half of every region — off by up to
/// `|twist| / 2`, `twist = h00 + h11 - h10 - h01`, with the maximum at the quad
/// centre. Over the player's own `Map/*/*.m` (42,799,104 quads in 4,644 files)
/// that was p50 0.12, p90 2.40, p99 11.97 and 2,009.8 units at worst on the
/// 21,399,552 affected quads; 21.6% of them were off by more than a unit.
///
/// Both orders below have positive signed area in XZ, which is the convention
/// [`BLOCK_INDICES`] is authored in (see [`canonical_block_indices`]).
const fn quad_triangles(v00: u32, v10: u32, v01: u32, v11: u32, even_parity: bool) -> [u32; 6] {
    if even_parity {
        [v01, v00, v11, v11, v00, v10]
    } else {
        [v01, v00, v10, v11, v01, v10]
    }
}

/// Triangle indices for one block's 17x17 vertex grid, row-major in z.
///
/// A block is 16 quads wide, so a block-local index has the same parity as its
/// region-local one and one array serves every block of the region.
pub const BLOCK_INDICES: [u32; 1536] = {
    let mut indices = [0u32; 1536];
    let mut z = 0;
    while z < 16 {
        let mut x = 0;
        while x < 16 {
            let v00 = (z * 17 + x) as u32;
            let tri = quad_triangles(v00, v00 + 1, v00 + 17, v00 + 18, (x & 1) == (z & 1));
            let at = (z * 16 + x) * 6;
            let mut k = 0;
            while k < 6 {
                indices[at + k] = tri[k];
                k += 1;
            }
            x += 1;
        }
        z += 1;
    }
    indices
};

impl JMXVMAPM {
    pub fn to_block_meshes(&self) -> Vec<Mesh> {
        self.blocks.iter().map(|block| Mesh::from(block)).collect()
    }
}

impl From<&TerrainBlock> for Mesh {
    fn from(block: &TerrainBlock) -> Self {
        let mut vertices = Vec::with_capacity(17 * 17);
        //let mut indices = Vec::with_capacity(16 * 16 * 6);
        let indices = BLOCK_INDICES.to_vec();
        let mut normals = Vec::with_capacity(17 * 17);
        let mut texture_ids = Vec::with_capacity(block.vertices.len());
        let mut texture_id_map = HashMap::with_capacity(17 * 17);
        let mut id: i32 = 0;

        for z in 0..17 {
            for x in 0..17 {
                let v = &block.vertices[z * 17 + x];
                let vx = x * 20;
                let vz = z * 20;
                let position = [vx as f32, v.height, vz as f32];
                vertices.push(position);
                if !texture_id_map.contains_key(&v.texture_id) {
                    texture_id_map.insert(v.texture_id, id);
                    id += 1;
                }
                let tex_index = texture_id_map.get(&v.texture_id).expect("i failed");
                texture_ids.push(*tex_index);

                let _size_factor = 1.0 / 1920.0;

                let current = Vec3::from(position);
                let mut normal = Vec3::ZERO;

                calc_normals(block, z, x, vx, vz, current, &mut normal);
                normals.push(normal.normalize().to_array());
            }
        }

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
        mesh.insert_indices(Indices::U32(indices));
        // Note: this single-block path (currently unused) still computes normals from the
        // block alone; the live `merge_block_meshes` path uses `region_normal`, which sees
        // across block borders.

        // let normals = mesh.attribute_mut(Mesh::ATTRIBUTE_NORMAL)
        //     .unwrap()
        //     .as_float3()
        //     .unwrap();
        //
        // let normals = normals.iter().map(|n| {
        //     (-Vec3::new(n[0], n[1], n[2])).to_array()
        // }).collect::<Vec<_>>();
        // mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);

        mesh
    }
}

/// Height of a vertex on the region-wide 97x97 grid (6x6 blocks of 17x17 vertices, where
/// adjacent blocks share their edge vertices), or `None` outside the region. This is what
/// lets normals see across block borders: the vertex one step past a block's edge lives in
/// the neighboring block, but on the region grid it is just `gx ± 1`.
fn region_vertex_height(map: &JMXVMAPM, gx: i32, gz: i32) -> Option<f32> {
    if !(0..=96).contains(&gx) || !(0..=96).contains(&gz) {
        return None;
    }
    let (bx, x) = if gx == 96 {
        (5, 16)
    } else {
        (gx / 16, gx % 16)
    };
    let (bz, z) = if gz == 96 {
        (5, 16)
    } else {
        (gz / 16, gz % 16)
    };
    let block = &map.blocks[(bz * 6 + bx) as usize];
    Some(block.vertices[(z * 17 + x) as usize].height)
}

/// Vertex normal on the region grid: the same sum of the four surrounding triangle face
/// normals `calc_normals` uses (identical winding, so interior vertices are unchanged),
/// but with neighbor heights taken from the whole region instead of a single block — no
/// more normal seams at block borders. Vertices on the region's outer edge still lack
/// neighbors (they live in a different `JMXVMAPM`), so region borders keep their seam.
fn region_normal(map: &JMXVMAPM, gx: i32, gz: i32) -> Vec3 {
    let current = Vec3::new(
        0.0,
        region_vertex_height(map, gx, gz).expect("vertex inside region"),
        0.0,
    );
    let w = region_vertex_height(map, gx - 1, gz).map(|h| Vec3::new(-20.0, h, 0.0));
    let e = region_vertex_height(map, gx + 1, gz).map(|h| Vec3::new(20.0, h, 0.0));
    let n = region_vertex_height(map, gx, gz - 1).map(|h| Vec3::new(0.0, h, -20.0));
    let s = region_vertex_height(map, gx, gz + 1).map(|h| Vec3::new(0.0, h, 20.0));

    let mut normal = Vec3::ZERO;
    if let (Some(w), Some(n)) = (w, n) {
        normal += calc_surface_normal(current, n, w);
    }
    if let (Some(w), Some(s)) = (w, s) {
        normal += calc_surface_normal(current, w, s);
    }
    if let (Some(e), Some(n)) = (e, n) {
        normal += calc_surface_normal(current, e, n);
    }
    if let (Some(e), Some(s)) = (e, s) {
        normal += calc_surface_normal(current, s, e);
    }
    normal.normalize()
}

/// Merges a rectangular group of blocks into one mesh (one draw call instead of one per
/// block). `blocks` is `(block, dx, dz)` in row-major group order, where `(dx, dz)` is the
/// block's offset *within the group*, in world units (`(block.x - group_origin_x) * 320.0`,
/// same for z) — i.e. positions are tiled at multiples of 320 starting from `(0,0)` for the
/// group's origin block, exactly like a single block's own local mesh space today.
/// `map` must be the region the blocks belong to; it provides the cross-block neighbor
/// heights for `region_normal`.
///
/// The caller is expected to give the merge-group *entity* the same
/// `Transform{translation: (group_origin_x * -320.0, 0.0, group_origin_z * 320.0), scale:
/// (-1.0, 1.0, 1.0)}` shape that an individual block's entity uses today, so the group's
/// mirroring is handled by Bevy's normal model-matrix machinery — vertex positions and
/// normals here are computed exactly as `impl From<&TerrainBlock> for Mesh` does, just
/// tiled across multiple blocks instead of baking a per-block offset into positions.
///
/// `reverse_winding` should come from the shared rule applied to that placement transform
/// (`needs_winding_reversal`): the mesh is built in canonical winding and reversed when the
/// placement is mirrored, so a `scale.x = -1` group renders exactly as it did before.
pub fn merge_block_meshes(
    map: &JMXVMAPM,
    blocks: &[(&TerrainBlock, f32, f32)],
    reverse_winding: bool,
) -> Mesh {
    let mut vertices = Vec::with_capacity(blocks.len() * 17 * 17);
    let mut normals = Vec::with_capacity(blocks.len() * 17 * 17);
    let mut indices = Vec::with_capacity(blocks.len() * BLOCK_INDICES.len());
    let canonical = canonical_block_indices();

    for (block_index, (block, dx, dz)) in blocks.iter().enumerate() {
        for z in 0..17 {
            for x in 0..17 {
                let v = &block.vertices[z * 17 + x];
                let vx = x * 20;
                let vz = z * 20;
                vertices.push([dx + vx as f32, v.height, dz + vz as f32]);

                let gx = block.x * 16 + x as i32;
                let gz = block.z * 16 + z as i32;
                normals.push(region_normal(map, gx, gz).to_array());
            }
        }

        let base = (block_index * 17 * 17) as u32;
        indices.extend(canonical.iter().map(|i| i + base));
    }

    if reverse_winding {
        reverse_winding_u32(&mut indices);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Region blocks per side (`JMXVMAPM` always holds a 6x6 grid).
const BLOCKS_PER_SIDE: i32 = 6;

/// A coarser [`merge_block_meshes`] for distance LOD, with optional skirts.
///
/// The idea: the splat shader derives everything it samples (tile map,
/// lightmap) from the world position, and the mesh carries only positions
/// and normals. A coarser grid over the same heights therefore textures
/// exactly like the full one, and only the silhouette of the relief loses
/// detail. `step` keeps every `step`-th vertex of each block's 17x17 grid
/// (1, 2 or 4 give 16x16, 8x8 or 4x4 quads per block), with the same split
/// rule and winding as the full mesh. On the coarse levels every kept vertex
/// has an even grid index, so [`quad_triangles`] always sees agreeing parity
/// and they keep the single h00-h11 diagonal they had.
///
/// Neighbouring regions can be drawn at different steps, and the coarser
/// edge then misses heights the finer one has, which opens a sliver between
/// them. `skirt` hangs a curtain of that depth straight down from every edge
/// of the region, on every level, so whichever side is higher covers the
/// gap. The curtain sits directly under the neighbour's surface, so it is
/// never seen otherwise. It is emitted with both windings, so it closes the
/// gap seen from either side.
pub fn merge_block_meshes_lod(
    map: &JMXVMAPM,
    blocks: &[(&TerrainBlock, f32, f32)],
    reverse_winding: bool,
    step: usize,
    skirt: Option<f32>,
) -> Mesh {
    assert!(
        matches!(step, 1 | 2 | 4),
        "step must divide the 16-quad block"
    );
    let n = 16 / step + 1;
    let mut vertices = Vec::with_capacity(blocks.len() * n * n);
    let mut normals = Vec::with_capacity(blocks.len() * n * n);
    let mut indices = Vec::new();

    for (block, dx, dz) in blocks {
        let base = vertices.len() as u32;
        let idx = |x: usize, z: usize| base + (z * n + x) as u32;
        for zi in 0..n {
            for xi in 0..n {
                let (x, z) = (xi * step, zi * step);
                let v = &block.vertices[z * 17 + x];
                vertices.push([dx + (x * 20) as f32, v.height, dz + (z * 20) as f32]);
                let (gx, gz) = (block.x * 16 + x as i32, block.z * 16 + z as i32);
                normals.push(region_normal(map, gx, gz).to_array());
            }
        }
        // the full mesh's split rule, on this level's grid; made canonical below
        for z in 0..n - 1 {
            for x in 0..n - 1 {
                let even_parity = ((x * step) & 1) == ((z * step) & 1);
                indices.extend(quad_triangles(
                    idx(x, z),
                    idx(x + 1, z),
                    idx(x, z + 1),
                    idx(x + 1, z + 1),
                    even_parity,
                ));
            }
        }

        let Some(depth) = skirt else { continue };
        // the edges of this block that are edges of the region
        let mut edges: Vec<Vec<(usize, usize)>> = Vec::new();
        if block.x == 0 {
            edges.push((0..n).map(|z| (0, z)).collect());
        }
        if block.x == BLOCKS_PER_SIDE - 1 {
            edges.push((0..n).map(|z| (n - 1, z)).collect());
        }
        if block.z == 0 {
            edges.push((0..n).map(|x| (x, 0)).collect());
        }
        if block.z == BLOCKS_PER_SIDE - 1 {
            edges.push((0..n).map(|x| (x, n - 1)).collect());
        }
        for edge in edges {
            let top_base = vertices.len() as u32;
            for &(x, z) in &edge {
                let top = vertices[idx(x, z) as usize];
                vertices.push([top[0], top[1] - depth, top[2]]);
                normals.push(normals[idx(x, z) as usize]);
            }
            for i in 0..edge.len() - 1 {
                let (a, b) = (idx(edge[i].0, edge[i].1), idx(edge[i + 1].0, edge[i + 1].1));
                let (a_low, b_low) = (top_base + i as u32, top_base + i as u32 + 1);
                // both windings: the curtain closes the gap seen from either side
                indices.extend([a, a_low, b, b, a_low, b_low]);
                indices.extend([a, b, a_low, b, b_low, a_low]);
            }
        }
    }

    // canonical winding (see `canonical_block_indices`), then the placement's
    reverse_winding_u32(&mut indices);
    if reverse_winding {
        reverse_winding_u32(&mut indices);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

fn calc_normals(
    block: &TerrainBlock,
    z: usize,
    x: usize,
    vx: usize,
    vz: usize,
    current: Vec3,
    mut normal_result: &mut Vec3,
) {
    if x > 0 {
        let w = Vec3::new(
            (vx - 20) as f32,
            block.vertices[z * 17 + x - 1].height,
            vz as f32,
        );
        calc_vertical_normals(block, z, x, vx, vz, current, &mut normal_result, w, false);
    }

    if x < 16 {
        let e = Vec3::new(
            (vx + 20) as f32,
            block.vertices[z * 17 + x + 1].height,
            vz as f32,
        );
        calc_vertical_normals(block, z, x, vx, vz, current, &mut normal_result, e, true);
    }
}

fn calc_vertical_normals(
    block: &TerrainBlock,
    z: usize,
    x: usize,
    vx: usize,
    vz: usize,
    current: Vec3,
    normal_result: &mut Vec3,
    h: Vec3,
    is_east: bool,
) {
    if z > 0 {
        let n = Vec3::new(
            vx as f32,
            block.vertices[(z - 1) * 17 + x].height,
            (vz - 20) as f32,
        );
        if is_east {
            *normal_result += calc_surface_normal(current, h, n);
        } else {
            *normal_result += calc_surface_normal(current, n, h);
        }
    }
    if z < 16 {
        let s = Vec3::new(
            vx as f32,
            block.vertices[(z + 1) * 17 + x].height,
            (vz + 20) as f32,
        );
        if is_east {
            *normal_result += calc_surface_normal(current, s, h);
        } else {
            *normal_result += calc_surface_normal(current, h, s);
        }
    }
}

// use BLOCK_INDICES instead
#[allow(dead_code)]
pub fn create_indices() -> Vec<u32> {
    let mut indices = Vec::with_capacity(16 * 16 * 6);

    for z in 0..16 {
        for x in 0..16 {
            let i0: u32 = x + z * 17;
            let i1: u32 = i0 + 1;
            let i2: u32 = i0 + 17;
            let i3: u32 = i2 + 1;
            /*

               i0      i1


               i2      i3

            */
            indices.push(i2);
            indices.push(i0);
            indices.push(i3);

            indices.push(i3);
            indices.push(i0);
            indices.push(i1);
        }
    }

    indices
}

fn calc_surface_normal(t1: Vec3, t2: Vec3, t3: Vec3) -> Vec3 {
    let v1 = t2 - t1;
    let v2 = t3 - t1;

    v1.cross(v2).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::m::{MapVertex, WaterType};
    use crate::assets::nvm::height_map::height_in_quad;
    use bevy::camera::primitives::Aabb;

    /// Four corners with a non-zero twist, so the two possible split diagonals
    /// give different heights at the quad centre: (10 + 25) / 2 = 17.5 across
    /// h00-h11, (13 + 17) / 2 = 15 across h10-h01.
    const CORNERS: [f32; 4] = [10.0, 13.0, 17.0, 25.0];

    /// The diagonal two triangles share, as the pair of corner roles
    /// (0 = h00, 1 = h10, 2 = h01, 3 = h11) they have in common.
    fn shared_diagonal(quad: &[u32], v00: u32) -> (usize, usize) {
        let role = |v: u32| match v - v00 {
            0 => 0,
            1 => 1,
            17 => 2,
            18 => 3,
            other => panic!("index {other} past the quad's corners"),
        };
        let (a, b) = (quad[..3].to_vec(), quad[3..].to_vec());
        let mut shared: Vec<usize> = a
            .iter()
            .filter(|v| b.contains(v))
            .map(|v| role(*v))
            .collect();
        shared.sort_unstable();
        assert_eq!(shared.len(), 2, "two triangles of a quad share an edge");
        (shared[0], shared[1])
    }

    /// The drawn surface and the walked surface have to be the same surface:
    /// every quad's split diagonal in [`BLOCK_INDICES`] must be the diagonal
    /// `nvm::height_map::height_in_quad` interpolates across. Read from the
    /// mesh by the edge its two triangles share, and from the height field by
    /// the value at the quad centre, which is the midpoint of the split.
    #[test]
    fn the_drawn_split_is_the_split_the_mover_walks_on() {
        let [h00, h10, h01, h11] = CORNERS;
        for z in 0..16usize {
            for x in 0..16usize {
                let v00 = (z * 17 + x) as u32;
                let at = (z * 16 + x) * 6;
                let drawn = shared_diagonal(&BLOCK_INDICES[at..at + 6], v00);
                let walked = height_in_quad(CORNERS, (x & 1) == (z & 1), 0.5, 0.5);
                let expected = match drawn {
                    (0, 3) => 0.5 * (h00 + h11),
                    (1, 2) => 0.5 * (h10 + h01),
                    other => panic!("quad ({x}, {z}) is not split along a diagonal: {other:?}"),
                };
                assert_eq!(
                    walked, expected,
                    "quad ({x}, {z}) is drawn across {drawn:?} but walked elsewhere"
                );
            }
        }
    }

    /// Every triangle keeps the authored winding, which the whole terrain
    /// pipeline depends on (see [`canonical_block_indices`]): positive signed
    /// area in XZ on the grid the indices address.
    #[test]
    fn both_split_directions_keep_the_authored_winding() {
        for tri in BLOCK_INDICES.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| ((i % 17) as f32, (i / 17) as f32));
            let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            assert!(area > 0.0, "triangle {tri:?} is wound the other way");
        }
    }

    /// Builds a full 6x6-block region whose vertex heights come from `height(gx, gz)`
    /// on the region-wide 97x97 grid, so adjacent blocks share edge heights exactly.
    fn test_region(height: impl Fn(i32, i32) -> f32) -> JMXVMAPM {
        let mut blocks = Vec::with_capacity(36);
        for bz in 0..6 {
            for bx in 0..6 {
                let mut vertices = Vec::with_capacity(17 * 17);
                for z in 0..17i32 {
                    for x in 0..17i32 {
                        vertices.push(MapVertex {
                            x,
                            z,
                            height: height(bx * 16 + x, bz * 16 + z),
                            texture_id: 0,
                            splat_scale: 0,
                            splat_offset: 0,
                            brightness: 0,
                        });
                    }
                }
                blocks.push(TerrainBlock {
                    x: bx,
                    z: bz,
                    flag: 0,
                    environment_id: 0,
                    water_type: WaterType::None,
                    vertices,
                    tiles: Vec::new(),
                    aabb: Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
                });
            }
        }
        JMXVMAPM {
            format: "JMXVMAPM".into(),
            version: "1000".into(),
            blocks,
        }
    }

    #[test]
    fn region_vertex_height_spans_blocks_and_shared_edges() {
        let map = test_region(|gx, gz| (gx * 1000 + gz) as f32);
        // Interior of the first block, a shared block edge (gx=16 lives in both block 0
        // and block 1), and the region's far corner.
        assert_eq!(region_vertex_height(&map, 3, 5), Some(3005.0));
        assert_eq!(region_vertex_height(&map, 16, 8), Some(16008.0));
        assert_eq!(region_vertex_height(&map, 96, 96), Some(96096.0));
        // Outside the region there is no data.
        assert_eq!(region_vertex_height(&map, -1, 0), None);
        assert_eq!(region_vertex_height(&map, 0, 97), None);
    }

    #[test]
    fn planar_slope_has_seamless_normals_across_block_borders() {
        // On a plane every triangle shares one normal, so every vertex normal — including
        // those on block borders, which previously only saw one block's half of the
        // neighborhood — must equal the plane normal.
        let (a, b) = (3.0, -7.0);
        let map = test_region(|gx, gz| a * gx as f32 + b * gz as f32);
        // Heights step by (a, b) per 20-unit grid step => normal ∝ (-a, 20, -b).
        let expected = Vec3::new(-a, 20.0, -b).normalize();
        for gz in 0..97 {
            for gx in 0..97 {
                let normal = region_normal(&map, gx, gz);
                assert!(
                    normal.abs_diff_eq(expected, 1e-5),
                    "normal at ({gx}, {gz}) is {normal:?}, expected {expected:?}"
                );
            }
        }
    }

    /// The region's blocks laid out as `load_terrain_system` passes them.
    fn region_blocks(map: &JMXVMAPM) -> Vec<(&TerrainBlock, f32, f32)> {
        map.blocks
            .iter()
            .map(|b| (b, (b.x * 320) as f32, (b.z * 320) as f32))
            .collect()
    }

    fn positions(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p.clone(),
            _ => panic!("positions"),
        }
    }

    fn indices(mesh: &Mesh) -> Vec<u32> {
        match mesh.indices() {
            Some(Indices::U32(i)) => i.clone(),
            _ => panic!("u32 indices"),
        }
    }

    /// At full resolution and without a skirt, the LOD builder is the
    /// ordinary merged mesh, vertex for vertex and triangle for triangle.
    #[test]
    fn full_step_lod_matches_the_merged_mesh() {
        let map = test_region(|gx, gz| (gx * 3 + gz * 7) as f32);
        let blocks = region_blocks(&map);
        for reverse in [false, true] {
            let full = merge_block_meshes(&map, &blocks, reverse);
            let lod = merge_block_meshes_lod(&map, &blocks, reverse, 1, None);
            assert_eq!(positions(&full), positions(&lod));
            assert_eq!(indices(&full), indices(&lod), "reverse_winding={reverse}");
        }
    }

    #[test]
    fn coarser_steps_keep_every_nth_vertex() {
        let map = test_region(|gx, gz| (gx + gz) as f32);
        let blocks = region_blocks(&map);
        for (step, per_side) in [(2, 9), (4, 5)] {
            let mesh = merge_block_meshes_lod(&map, &blocks, false, step, None);
            assert_eq!(positions(&mesh).len(), 36 * per_side * per_side);
            let quads = (per_side - 1) * (per_side - 1);
            assert_eq!(indices(&mesh).len(), 36 * quads * 6);
            // every kept vertex lies on the full grid, at its own height
            for p in positions(&mesh) {
                assert_eq!(p[0] % (20.0 * step as f32), 0.0);
                assert_eq!(p[1], (p[0] / 20.0 + p[2] / 20.0));
            }
        }
    }

    /// The skirt hangs only from the region's outer edges, straight down.
    #[test]
    fn the_skirt_hangs_from_the_region_edge() {
        let map = test_region(|_, _| 100.0);
        let blocks = region_blocks(&map);
        let bare = positions(&merge_block_meshes_lod(&map, &blocks, false, 4, None));
        let skirted = positions(&merge_block_meshes_lod(&map, &blocks, false, 4, Some(50.0)));
        // 20 edge blocks, the 4 corner ones with two region edges each: 24
        // edges of 5 vertices at the quarter grid
        assert_eq!(skirted.len() - bare.len(), 24 * 5);
        let low: Vec<_> = skirted.iter().filter(|p| p[1] == 50.0).collect();
        assert_eq!(low.len(), 24 * 5);
        for p in low {
            let on_edge = p[0] == 0.0 || p[0] == 1920.0 || p[2] == 0.0 || p[2] == 1920.0;
            assert!(on_edge, "{p:?} is not on the region edge");
        }
    }
}
