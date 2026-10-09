//! Texture detail (`graphics.texture_detail`): drop the largest mip levels of
//! world textures at load, to fit a 1-2 GB card.
//!
//! A texture's top mip level holds three quarters of its bytes, so starting
//! the chain one level down cuts its VRAM by ~75%, and two levels by ~94%. The
//! smaller levels are already in every DDJ (authored mip chains), so this is a
//! slice rather than a resample: no quality is lost beyond the resolution
//! itself, and no load time is spent.
//!
//! It applies to model textures (`data://prim/...`: buildings, props,
//! characters, equipment) and to the ground-tile arrays. The HUD's textures
//! are left whole, since a blurred interface is never worth the bytes, and so
//! are effects (small already) and the skybox.
//!
//! The level is read by the DDJ loader, which runs on the async task pool,
//! through a shared atomic that the client sets from config
//! (`assets::apply_texture_detail`; this module is also part of the parser
//! library, so it does not see the config). Textures keep the level they were
//! loaded with, so a change reaches what loads afterwards. For everything,
//! that means a restart.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::asset::AssetPath;
use bevy::prelude::*;

/// Mip levels to drop from the top of world textures, shared with the DDJ
/// loader.
#[derive(Resource, Clone, Default)]
pub struct TextureDetailLevel(pub Arc<AtomicU32>);

impl TextureDetailLevel {
    pub fn skipped_mips(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }

    pub fn set(&self, skipped_mips: u32) {
        self.0.store(skipped_mips, Ordering::Relaxed);
    }
}

/// Whether `path` names a texture texture detail applies to: a model texture
/// under `data://prim/`.
pub fn applies_to(path: &AssetPath) -> bool {
    path.source().as_str() == Some("data")
        && path
            .path()
            .to_string_lossy()
            .to_ascii_lowercase()
            .replace('\\', "/")
            .starts_with("prim/")
}

/// Drop up to `skip` top mip levels of a single-layer 2D `image`, keeping at
/// least one level. Compressed formats only lose a level while the new top
/// level stays a whole number of blocks, as wgpu requires. Returns the levels
/// actually dropped.
pub fn drop_top_mips(image: &mut Image, skip: u32) -> u32 {
    let desc = &image.texture_descriptor;
    if skip == 0 || desc.size.depth_or_array_layers != 1 {
        return 0;
    }
    let format = desc.format;
    let (block_w, block_h) = format.block_dimensions();
    let Some(block_bytes) = format.block_copy_size(None) else {
        return 0;
    };
    let (mut width, mut height) = (desc.size.width, desc.size.height);
    let mut levels = desc.mip_level_count;
    let mut dropped = 0;
    let mut offset = 0usize;
    while dropped < skip && levels > 1 {
        let (next_w, next_h) = ((width / 2).max(1), (height / 2).max(1));
        if next_w % block_w != 0 || next_h % block_h != 0 {
            break;
        }
        offset += level_len(width, height, block_w, block_h, block_bytes);
        width = next_w;
        height = next_h;
        levels -= 1;
        dropped += 1;
    }
    if dropped == 0 {
        return 0;
    }
    let Some(data) = image.data.as_mut() else {
        return 0;
    };
    if offset > data.len() {
        return 0;
    }
    data.drain(..offset);
    image.texture_descriptor.size.width = width;
    image.texture_descriptor.size.height = height;
    image.texture_descriptor.mip_level_count = levels;
    dropped
}

/// Bytes of one mip level of `width` x `height` in a format of
/// `block_w` x `block_h` blocks of `block_bytes` each.
fn level_len(width: u32, height: u32, block_w: u32, block_h: u32, block_bytes: u32) -> usize {
    (width.div_ceil(block_w) * height.div_ceil(block_h) * block_bytes) as usize
}

/// [`drop_top_mips`]'s level arithmetic, for a format the caller lays out by
/// hand (the BC1 tile arrays): the bytes of the top `skip` levels of a
/// `size` x `size` BC1 chain.
pub fn bc1_top_levels_len(size: u32, skip: u32) -> usize {
    (0..skip)
        .map(|level| level_len(size >> level, size >> level, 4, 4, 8))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    fn chain(width: u32, height: u32, levels: u32, format: TextureFormat) -> Image {
        let (bw, bh) = format.block_dimensions();
        let bytes = format.block_copy_size(None).unwrap();
        let len: usize = (0..levels)
            .map(|l| level_len((width >> l).max(1), (height >> l).max(1), bw, bh, bytes))
            .sum();
        let mut image = Image::new_uninit(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            format,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_descriptor.mip_level_count = levels;
        // tag each level's bytes with its index, to see which survive
        let mut data = Vec::with_capacity(len);
        for l in 0..levels {
            let n = level_len((width >> l).max(1), (height >> l).max(1), bw, bh, bytes);
            data.extend(std::iter::repeat_n(l as u8, n));
        }
        image.data = Some(data);
        image
    }

    #[test]
    fn dropping_a_level_keeps_the_rest_of_the_chain() {
        let mut image = chain(256, 128, 9, TextureFormat::Bc1RgbaUnormSrgb);
        assert_eq!(drop_top_mips(&mut image, 1), 1);
        let desc = &image.texture_descriptor;
        assert_eq!((desc.size.width, desc.size.height), (128, 64));
        assert_eq!(desc.mip_level_count, 8);
        assert_eq!(image.data.as_ref().unwrap()[0], 1, "level 1 is now first");
    }

    #[test]
    fn compressed_chains_stop_at_whole_blocks() {
        // 8x8 BC1: 4x4 is still one whole block, 2x2 is not
        let mut image = chain(8, 8, 4, TextureFormat::Bc1RgbaUnormSrgb);
        assert_eq!(drop_top_mips(&mut image, 3), 1);
        assert_eq!(image.texture_descriptor.size.width, 4);
    }

    #[test]
    fn a_single_level_is_never_dropped() {
        let mut image = chain(64, 64, 1, TextureFormat::Rgba8UnormSrgb);
        assert_eq!(drop_top_mips(&mut image, 2), 0);
        assert_eq!(image.texture_descriptor.size.width, 64);
    }

    #[test]
    fn uncompressed_chains_drop_freely() {
        let mut image = chain(64, 32, 7, TextureFormat::Rgba8UnormSrgb);
        assert_eq!(drop_top_mips(&mut image, 2), 2);
        assert_eq!(image.texture_descriptor.size.width, 16);
        assert_eq!(image.data.as_ref().unwrap()[0], 2);
    }

    #[test]
    fn only_model_textures_are_affected() {
        assert!(applies_to(&AssetPath::parse(
            "data://prim/mtrl/bldg/china/house.ddj"
        )));
        assert!(!applies_to(&AssetPath::parse(
            "media://interface/ifcommon/com_frame.ddj"
        )));
        assert!(!applies_to(&AssetPath::parse("map://tile2d/grass.ddj")));
    }

    #[test]
    fn bc1_top_levels_len_matches_the_layer_layout() {
        // 512 BC1: 128*128 blocks * 8 bytes
        assert_eq!(bc1_top_levels_len(512, 1), 131_072);
        assert_eq!(bc1_top_levels_len(512, 2), 131_072 + 32_768);
    }
}
