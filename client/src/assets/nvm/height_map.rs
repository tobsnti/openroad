use bytes::Buf;
use std::io::Cursor;

const HEIGHT_MAP_VERTS_PER_SIDE: usize = 97;
const HEIGHT_MAP_SIZE: usize = HEIGHT_MAP_VERTS_PER_SIDE * HEIGHT_MAP_VERTS_PER_SIDE;
/// World-space distance between adjacent height samples (97 verts spanning a
/// 1920-unit region).
const HEIGHT_MAP_STEP: f32 = 20.0;

#[derive(Clone)]
pub struct HeightMap(Vec<f32>);

impl HeightMap {
    /// A region whose terrain sits at a single height everywhere. Test-only:
    /// real height maps come from the `.nvm` byte stream.
    #[cfg(test)]
    pub(crate) fn flat(height: f32) -> Self {
        Self(vec![height; HEIGHT_MAP_SIZE])
    }

    /// Terrain height at a region-local (x, z) position in [0, 1920], read the
    /// way the original reads it. Positions outside the region are clamped to
    /// its border.
    ///
    /// Idea: a quad's four corner heights are almost never coplanar, so the quad
    /// has no one surface — a surface has to be chosen, and the choice is
    /// visible, because the terrain is *drawn* as triangles. The original
    /// therefore splits every quad into two triangles and interpolates inside
    /// the one that contains the position, and the direction of the split
    /// alternates like a checkerboard: `sro_client.exe` `FindHeight`
    /// (VA 0x004051B0) compares `ix % 2` with `iz % 2` (0x004051F6 / 0x00405246)
    /// and then picks the triangle from `tz` vs `tx` where the parities agree
    /// (split h00-h11), or from `tx + tz` vs 1 where they differ (split
    /// h10-h01). Its grid constants are the two above: stride 97
    /// (`imul $0x61` at 0x004051E5) and cell size 20.0 (the double at
    /// 0x00E1B830).
    ///
    /// This was bilinear before. A bilinear patch is a different, curved
    /// surface: it deviates from the triangulated one by up to `|twist| / 4`,
    /// `twist = h00 + h11 - h10 - h01`, with the maximum at the quad's centre.
    /// Over the player's own `Map/*/*.m` (1,105,920 quads) that is a median of
    /// 0.05 units, 1.19 at the 90th percentile and 6.71 at the 99th — a third
    /// of a body height (~18 units) on broken ground, and it applied to *every*
    /// quad, because a curved patch meets flat triangles nowhere but their
    /// shared corners and edges.
    ///
    /// Our own terrain mesh still splits every quad along h00-h11
    /// (`assets::m::block_mesh::BLOCK_INDICES`), so after this the mover stands
    /// exactly on the drawn surface wherever the parities agree — half the
    /// quads — and the other half needs the mesh to alternate too.
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let max_cell = (HEIGHT_MAP_VERTS_PER_SIDE - 2) as f32;
        let fx = (x / HEIGHT_MAP_STEP).clamp(0.0, max_cell + 1.0);
        let fz = (z / HEIGHT_MAP_STEP).clamp(0.0, max_cell + 1.0);
        let x0 = (fx.floor() as usize).min(HEIGHT_MAP_VERTS_PER_SIDE - 2);
        let z0 = (fz.floor() as usize).min(HEIGHT_MAP_VERTS_PER_SIDE - 2);
        let tx = fx - x0 as f32;
        let tz = fz - z0 as f32;
        let h = |xi: usize, zi: usize| self.0[zi * HEIGHT_MAP_VERTS_PER_SIDE + xi];
        let (h00, h10) = (h(x0, z0), h(x0 + 1, z0));
        let (h01, h11) = (h(x0, z0 + 1), h(x0 + 1, z0 + 1));
        if (x0 & 1) == (z0 & 1) {
            if tz >= tx {
                h00 + tz * (h01 - h00) + tx * (h11 - h01)
            } else {
                h00 + tx * (h10 - h00) + tz * (h11 - h10)
            }
        } else if tx + tz <= 1.0 {
            h00 + tx * (h10 - h00) + tz * (h01 - h00)
        } else {
            h11 + (1.0 - tx) * (h01 - h11) + (1.0 - tz) * (h10 - h11)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map whose quad (`x0`, `z0`) carries the four given corners; every other
    /// vertex is 0. The caller queries inside that quad.
    fn quad(x0: usize, z0: usize, corners: [f32; 4]) -> HeightMap {
        let mut v = vec![0.0; HEIGHT_MAP_SIZE];
        let [h00, h10, h01, h11] = corners;
        v[z0 * HEIGHT_MAP_VERTS_PER_SIDE + x0] = h00;
        v[z0 * HEIGHT_MAP_VERTS_PER_SIDE + x0 + 1] = h10;
        v[(z0 + 1) * HEIGHT_MAP_VERTS_PER_SIDE + x0] = h01;
        v[(z0 + 1) * HEIGHT_MAP_VERTS_PER_SIDE + x0 + 1] = h11;
        HeightMap(v)
    }

    const CORNERS: [f32; 4] = [10.0, 13.0, 17.0, 25.0];

    #[test]
    fn corners_are_the_corner_heights() {
        let map = quad(2, 2, CORNERS);
        let (x, z) = (2.0 * HEIGHT_MAP_STEP, 2.0 * HEIGHT_MAP_STEP);
        assert_eq!(map.height_at(x, z), 10.0);
        assert_eq!(map.height_at(x + HEIGHT_MAP_STEP, z), 13.0);
        assert_eq!(map.height_at(x, z + HEIGHT_MAP_STEP), 17.0);
        assert_eq!(
            map.height_at(x + HEIGHT_MAP_STEP, z + HEIGHT_MAP_STEP),
            25.0
        );
    }

    /// The centre of a quad is the midpoint of the split diagonal, and which
    /// diagonal that is alternates with the parity of the quad's indices. This
    /// is the one query that separates the three candidate surfaces: bilinear
    /// would give the average of all four corners (16.25) in both quads.
    #[test]
    fn centre_sits_on_the_alternating_split_diagonal() {
        let half = HEIGHT_MAP_STEP / 2.0;
        let centre = |x0: usize, z0: usize| {
            quad(x0, z0, CORNERS).height_at(
                x0 as f32 * HEIGHT_MAP_STEP + half,
                z0 as f32 * HEIGHT_MAP_STEP + half,
            )
        };
        // Parities agree: split h00-h11, centre = (10 + 25) / 2.
        assert_eq!(centre(2, 2), 17.5);
        assert_eq!(centre(3, 3), 17.5);
        // Parities differ: split h10-h01, centre = (13 + 17) / 2.
        assert_eq!(centre(2, 3), 15.0);
        assert_eq!(centre(3, 2), 15.0);
    }

    /// Both triangles of a quad must agree along their shared edge, or a mover
    /// crossing it steps.
    #[test]
    fn the_split_edge_is_continuous() {
        for (x0, z0) in [(2, 2), (2, 3)] {
            let map = quad(x0, z0, CORNERS);
            let (ox, oz) = (x0 as f32 * HEIGHT_MAP_STEP, z0 as f32 * HEIGHT_MAP_STEP);
            for i in 1..20 {
                let t = i as f32 / 20.0;
                let on_diagonal = if (x0 & 1) == (z0 & 1) { t } else { 1.0 - t };
                let from_below = map.height_at(
                    ox + t * HEIGHT_MAP_STEP,
                    oz + (on_diagonal - 1e-4) * HEIGHT_MAP_STEP,
                );
                let from_above = map.height_at(
                    ox + t * HEIGHT_MAP_STEP,
                    oz + (on_diagonal + 1e-4) * HEIGHT_MAP_STEP,
                );
                assert!(
                    (from_below - from_above).abs() < 0.01,
                    "quad ({x0}, {z0}) jumps at t={t}: {from_below} vs {from_above}"
                );
            }
        }
    }

    #[test]
    fn positions_outside_the_region_clamp_to_the_border() {
        let map = quad(0, 0, CORNERS);
        assert_eq!(map.height_at(-100.0, -100.0), map.height_at(0.0, 0.0));
        let far = HEIGHT_MAP_VERTS_PER_SIDE as f32 * HEIGHT_MAP_STEP * 2.0;
        assert_eq!(map.height_at(far, far), 0.0);
    }
}

impl From<&mut Cursor<&[u8]>> for HeightMap {
    fn from(cursor: &mut Cursor<&[u8]>) -> Self {
        let mut height_map: Vec<f32> = Vec::with_capacity(HEIGHT_MAP_SIZE);
        for _ in 0..HEIGHT_MAP_SIZE {
            let height = cursor.get_f32_le();
            height_map.push(height);
        }
        Self(height_map)
    }
}
