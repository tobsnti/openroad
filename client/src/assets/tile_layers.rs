//! Ground tiles as layers of the terrain texture arrays.
//!
//! Idea: the splat shader used to index a *binding array* of tile textures
//! per fragment — bindless sampling that GPUs from before ~2016 (Haswell-era
//! Intel, older NVIDIA/AMD binding tiers) and WebGPU/WebGL2-class devices do
//! not offer, so the client could not start on them at all. A
//! `texture_2d_array` indexed per fragment works on every GPU, but all its
//! layers must share one size, format and mip count. The shipped tiles nearly
//! do (Map.pk2 census 2026-10-05: 741 of 752 are 512x512 BC1 with a full
//! 10-level chain; 10 are 256x256 with 9 levels; 1 is 512x512 without mips),
//! so each tile is normalized to [`TILE_LAYER_SIZE`] / [`TILE_LAYER_MIPS`]
//! here, at load, while the loader still owns its bytes.
//!
//! Normalizing stays in the compressed format where it can: a 2x
//! nearest-neighbour upscale of BC1 is exact block by block (every 2x2 group
//! of output texels lies inside one source block, so the block keeps its
//! endpoints and its 2-bit indices are duplicated). Levels the source lacks
//! are box-filtered from the level above and re-encoded with a small
//! range-fit encoder — that only runs for the odd tile out.
//!
//! The 10-bit tile id space (`TILE_SLOT_COUNT` = 1024) maps onto
//! [`TILE_ARRAY_COUNT`] arrays of [`TILE_ARRAY_LAYERS`] layers: 256 is the
//! array-layer limit baseline WebGPU/WebGL2 guarantee.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use bevy::asset::AssetPath;
use bevy::prelude::*;

use crate::assets::ddj::decode_dxt1_rgba8;

/// Edge length of every tile layer.
pub const TILE_LAYER_SIZE: u32 = 512;
/// Mip levels of every tile layer: 512 down to 1.
pub const TILE_LAYER_MIPS: u32 = TILE_LAYER_SIZE.ilog2() + 1;
/// Layers per tile array — the guaranteed baseline `max_texture_array_layers`.
pub const TILE_ARRAY_LAYERS: u32 = 256;
/// Arrays needed to cover the 10-bit tile id space.
pub const TILE_ARRAY_COUNT: usize = 4;

/// Bytes of one BC1 mip level of a square `size` texture (one 8-byte block
/// per 4x4 texels, at least one block).
pub fn bc1_level_len(size: u32) -> usize {
    (size.div_ceil(4) * size.div_ceil(4) * 8) as usize
}

/// Bytes of one normalized tile layer (all its mips).
pub fn tile_layer_len() -> usize {
    (0..TILE_LAYER_MIPS)
        .map(|level| bc1_level_len(TILE_LAYER_SIZE >> level))
        .sum()
}

/// Array and layer a tile id lives in.
pub fn tile_array_slot(tile_id: u16) -> (usize, u32) {
    (
        tile_id as usize / TILE_ARRAY_LAYERS as usize,
        tile_id as u32 % TILE_ARRAY_LAYERS,
    )
}

/// A white layer for ids without a texture (matching the white fallback
/// image they used to bind): every block `c0 == c1 == 0xffff`, index 0.
pub fn white_tile_layer() -> Vec<u8> {
    const WHITE_BLOCK: [u8; 8] = [0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0];
    WHITE_BLOCK.repeat(tile_layer_len() / 8)
}

/// Normalizes a square, power-of-two BC1 tile — `mips` levels laid out
/// largest first in `data` — to a [`TILE_LAYER_SIZE`] layer with
/// [`TILE_LAYER_MIPS`] levels. `None` for shapes a layer cannot hold
/// (non-square, non-power-of-two, larger than a layer, truncated data).
pub fn normalize_bc1_tile(width: u32, height: u32, mips: u32, data: &[u8]) -> Option<Vec<u8>> {
    if width != height || !width.is_power_of_two() || width > TILE_LAYER_SIZE || mips == 0 {
        return None;
    }
    // the source's own levels, largest first
    let mut source = Vec::new();
    let mut offset = 0;
    for level in 0..mips.min(width.ilog2() + 1) {
        let len = bc1_level_len(width >> level);
        source.push(data.get(offset..offset + len)?.to_vec());
        offset += len;
    }

    let mut layer = Vec::with_capacity(tile_layer_len());
    let mut previous: Option<Vec<u8>> = None;
    for level in 0..TILE_LAYER_MIPS {
        let size = TILE_LAYER_SIZE >> level;
        let bytes = if size > width {
            // above the source: upscale its top level (exact in BC1)
            let mut up = source[0].clone();
            let mut up_size = width;
            while up_size < size {
                up = upscale_bc1_2x(&up, up_size);
                up_size *= 2;
            }
            up
        } else {
            let source_level = (width / size).ilog2() as usize;
            match source.get(source_level) {
                Some(bytes) => bytes.clone(),
                // below the source's chain: filter the level above
                None => downsample_bc1(previous.as_ref()?, size * 2),
            }
        };
        layer.extend_from_slice(&bytes);
        previous = Some(bytes);
    }
    Some(layer)
}

/// 2x nearest-neighbour upscale of one BC1 level of a square `size` texture.
fn upscale_bc1_2x(level: &[u8], size: u32) -> Vec<u8> {
    let out_size = size * 2;
    let (blocks_in, blocks_out) = (size.div_ceil(4) as usize, out_size.div_ceil(4) as usize);
    let mut out = vec![0u8; blocks_out * blocks_out * 8];
    for oy in 0..blocks_out {
        for ox in 0..blocks_out {
            // the 2x2 output texels starting at (2x, 2y) all come from source
            // texel (x, y); a 4x4 output block's sources (2x2 texels) never
            // straddle a source block boundary
            let (sx, sy) = ((ox * 4) / 2, (oy * 4) / 2);
            let (bx, by) = (sx / 4, sy / 4);
            let src = &level[(by * blocks_in + bx) * 8..][..8];
            let src_indices = u32::from_le_bytes([src[4], src[5], src[6], src[7]]);
            let mut indices = 0u32;
            for t in 0..16 {
                let (tx, ty) = (t % 4, t / 4);
                let (px, py) = ((sx + tx / 2) % 4, (sy + ty / 2) % 4);
                let index = (src_indices >> ((py * 4 + px) * 2)) & 3;
                indices |= index << (t * 2);
            }
            let dst = &mut out[(oy * blocks_out + ox) * 8..][..8];
            dst[..4].copy_from_slice(&src[..4]);
            dst[4..].copy_from_slice(&indices.to_le_bytes());
        }
    }
    out
}

/// The next smaller mip of one BC1 level of a square `size` texture: decode,
/// 2x2 box filter, re-encode.
fn downsample_bc1(level: &[u8], size: u32) -> Vec<u8> {
    let rgba = decode_dxt1_rgba8(level, size, size);
    let (src, half) = (size as usize, (size / 2).max(1) as usize);
    let mut small = vec![0u8; half * half * 4];
    for y in 0..half {
        for x in 0..half {
            for c in 0..4 {
                let mut sum = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (px, py) = ((x * 2 + dx).min(src - 1), (y * 2 + dy).min(src - 1));
                    sum += rgba[(py * src + px) * 4 + c] as u32;
                }
                small[(y * half + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    encode_bc1_rgb(&small, half as u32)
}

/// Opaque BC1 range-fit encoder: per block, the RGB bounding box's corners as
/// endpoints, each texel the nearest of the four palette colours. Quality is
/// plain but only ever used for levels a tile was shipped without.
fn encode_bc1_rgb(rgba: &[u8], size: u32) -> Vec<u8> {
    let to565 = |c: [u8; 3]| -> u16 {
        ((c[0] as u16 >> 3) << 11) | ((c[1] as u16 >> 2) << 5) | (c[2] as u16 >> 3)
    };
    let from565 = |c: u16| -> [i32; 3] {
        let (r, g, b) = ((c >> 11) & 0x1f, (c >> 5) & 0x3f, c & 0x1f);
        [
            (r as i32 * 255) / 31,
            (g as i32 * 255) / 63,
            (b as i32 * 255) / 31,
        ]
    };
    let blocks = size.div_ceil(4) as usize;
    let s = size as usize;
    let mut out = Vec::with_capacity(blocks * blocks * 8);
    for by in 0..blocks {
        for bx in 0..blocks {
            let texel = |t: usize| -> [u8; 3] {
                let (x, y) = ((bx * 4 + t % 4).min(s - 1), (by * 4 + t / 4).min(s - 1));
                let o = (y * s + x) * 4;
                [rgba[o], rgba[o + 1], rgba[o + 2]]
            };
            let (mut lo, mut hi) = ([255u8; 3], [0u8; 3]);
            for t in 0..16 {
                let c = texel(t);
                for i in 0..3 {
                    lo[i] = lo[i].min(c[i]);
                    hi[i] = hi[i].max(c[i]);
                }
            }
            let (mut c0, mut c1) = (to565(hi), to565(lo));
            if c0 < c1 {
                std::mem::swap(&mut c0, &mut c1);
            }
            let mut indices = 0u32;
            if c0 != c1 {
                // four-colour mode (c0 > c1)
                let (e0, e1) = (from565(c0), from565(c1));
                let palette = [
                    e0,
                    e1,
                    [0, 1, 2].map(|i| (2 * e0[i] + e1[i]) / 3),
                    [0, 1, 2].map(|i| (e0[i] + 2 * e1[i]) / 3),
                ];
                for t in 0..16 {
                    let c = texel(t).map(|v| v as i32);
                    let best = (0..4)
                        .min_by_key(|&p| (0..3).map(|i| (c[i] - palette[p][i]).pow(2)).sum::<i32>())
                        .unwrap_or(0);
                    indices |= (best as u32) << (t * 2);
                }
            }
            out.extend_from_slice(&c0.to_le_bytes());
            out.extend_from_slice(&c1.to_le_bytes());
            out.extend_from_slice(&indices.to_le_bytes());
        }
    }
    out
}

/// Normalized tile layers by asset path, filled by the DDJ loader while it
/// still owns the bytes and drained once the arrays are built.
#[derive(Resource, Clone, Default)]
pub struct TerrainTileLayers(pub Arc<RwLock<HashMap<AssetPath<'static>, Arc<Vec<u8>>>>>);

#[cfg(test)]
mod tests {
    use super::*;

    /// One BC1 block: endpoints and 16 two-bit indices.
    fn block(c0: u16, c1: u16, indices: u32) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[..2].copy_from_slice(&c0.to_le_bytes());
        b[2..4].copy_from_slice(&c1.to_le_bytes());
        b[4..].copy_from_slice(&indices.to_le_bytes());
        b
    }

    /// A full BC1 chain for a square `size` texture where every level's
    /// blocks are `fill`.
    fn chain(size: u32, mips: u32, fill: [u8; 8]) -> Vec<u8> {
        (0..mips)
            .flat_map(|l| fill.repeat(bc1_level_len(size >> l) / 8))
            .collect()
    }

    #[test]
    fn layer_shape_matches_the_shipped_tiles() {
        assert_eq!(TILE_LAYER_MIPS, 10);
        assert_eq!(TILE_ARRAY_COUNT as u32 * TILE_ARRAY_LAYERS, 1024);
        assert_eq!(tile_array_slot(0), (0, 0));
        assert_eq!(tile_array_slot(255), (0, 255));
        assert_eq!(tile_array_slot(256), (1, 0));
        assert_eq!(tile_array_slot(1023), (3, 255));
    }

    #[test]
    fn a_full_size_tile_passes_through_unchanged() {
        let data = chain(512, 10, block(0xf800, 0x001f, 0x1b1b_1b1b));
        let layer = normalize_bc1_tile(512, 512, 10, &data).unwrap();
        assert_eq!(layer, data);
    }

    #[test]
    fn upscaling_is_exact_nearest_neighbour() {
        // an 8x8 level: 2x2 blocks with distinct endpoints and index patterns
        let blocks = [
            block(0xf800, 0x0000, 0x0000_0000),
            block(0x07e0, 0x0000, 0x5555_5555),
            block(0x001f, 0x0000, 0xaaaa_aaaa),
            block(0xffff, 0x0000, 0xe4e4_e4e4),
        ];
        let level: Vec<u8> = blocks.concat();
        let up = upscale_bc1_2x(&level, 8);
        let (src, dst) = (
            decode_dxt1_rgba8(&level, 8, 8),
            decode_dxt1_rgba8(&up, 16, 16),
        );
        for y in 0..16 {
            for x in 0..16 {
                let d = &dst[(y * 16 + x) * 4..][..4];
                let s = &src[((y / 2) * 8 + x / 2) * 4..][..4];
                assert_eq!(d, s, "texel ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_half_size_tile_is_upscaled_and_keeps_its_chain_below() {
        let data = chain(256, 9, block(0x001f, 0x0000, 0x0000_0000));
        let layer = normalize_bc1_tile(256, 256, 9, &data).unwrap();
        assert_eq!(layer.len(), tile_layer_len());
        // levels 1.. are the source's own levels 0..
        let top = bc1_level_len(512);
        assert_eq!(&layer[top..], &data[..]);
    }

    #[test]
    fn missing_mips_are_generated() {
        let data = block(0x7bef, 0x7bef, 0).repeat(bc1_level_len(512) / 8);
        let layer = normalize_bc1_tile(512, 512, 1, &data).unwrap();
        assert_eq!(layer.len(), tile_layer_len());
        // a uniform grey tile stays uniform grey all the way down
        let mut offset = 0;
        for level in 0..TILE_LAYER_MIPS {
            let size = TILE_LAYER_SIZE >> level;
            let len = bc1_level_len(size);
            let rgba = decode_dxt1_rgba8(&layer[offset..offset + len], size, size);
            let grey = &rgba[..4];
            assert!(grey[0] > 100 && grey[0] < 140, "level {level}: {grey:?}");
            assert!(
                rgba.chunks(4).all(|t| t == grey),
                "level {level} not uniform"
            );
            offset += len;
        }
    }

    #[test]
    fn unusable_shapes_are_rejected() {
        assert!(normalize_bc1_tile(512, 256, 1, &[]).is_none());
        assert!(normalize_bc1_tile(1024, 1024, 1, &[]).is_none());
        assert!(normalize_bc1_tile(512, 512, 10, &[0; 16]).is_none());
    }
}

#[cfg(test)]
mod census {
    /// Every shipped ground tile must normalize into a layer.
    #[test]
    #[ignore = "reads the user's Map.pk2; requires local config.yaml key and assets"]
    fn every_shipped_tile_normalizes() {
        use crate::assets::ddj::JMXVDDJ;
        use bevy_pk2::prelude::{Archive, Pk2Key};
        use bytes::Bytes;
        use std::collections::BTreeMap;
        use std::path::Path;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let key = Pk2Key::resolve_from(&root.join("config.yaml")).unwrap();
        let archive = Archive::open(root.join("assets").join("Map.pk2"), &key).unwrap();
        let mut census: BTreeMap<String, usize> = BTreeMap::new();
        for (path, entry) in archive.root.get_all_entries() {
            let p = path.to_string_lossy().to_ascii_lowercase();
            if !entry.is_file() || !p.contains("tile2d") || !p.ends_with(".ddj") {
                continue;
            }
            let bytes = archive.read_file_bytes(&path).unwrap();
            let key = match JMXVDDJ::try_from(&mut Bytes::from(bytes)) {
                Ok(ddj) => {
                    if ddj.tile_layer().is_some() {
                        "ok".to_string()
                    } else {
                        "FAILED".to_string()
                    }
                }
                Err(_) => "unparsable".into(),
            };
            *census.entry(key).or_default() += 1;
        }
        for (k, n) in &census {
            println!("{n:5}  {k}");
        }
        assert!(!census.keys().any(|k| k.contains("FAILED")));
    }
}
