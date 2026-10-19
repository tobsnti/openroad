//! The ground-tile texture arrays the splat shader samples (see
//! `assets::tile_layers` for why arrays, and how each tile became a layer).
//!
//! Built exactly once, as soon as every tile `tile2d.ifo` defines has been
//! loaded (its layer is in [`TerrainTileLayers`]) or has failed to load:
//! four `texture_2d_array`s of up to [`TILE_ARRAY_LAYERS`] layers, layer
//! `id % 256` of array `id / 256` holding tile `id`. Ids without a usable
//! texture get a white layer, as their fallback image was white before. The
//! layer table is drained afterwards; it only held the bytes until now.

use bevy::asset::{LoadState, RenderAssetUsages};
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::render::RenderApp;

use super::block_splat_material::TerrainTileAtlas;
use crate::assets::tile_layers::{
    tile_layer_len, white_tile_layer, TerrainTileLayers, TILE_ARRAY_COUNT, TILE_ARRAY_LAYERS,
    TILE_LAYER_MIPS, TILE_LAYER_SIZE,
};

/// The four ground-tile arrays, indexed by `tile_id / 256`.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct TerrainTileArrays(pub [Handle<Image>; TILE_ARRAY_COUNT]);

pub(super) fn build_tile_arrays(
    mut commands: Commands,
    atlas: Option<Res<TerrainTileAtlas>>,
    layers: Res<TerrainTileLayers>,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    texture_detail: Option<Res<crate::assets::texture_detail::TextureDetailLevel>>,
) {
    let Some(atlas) = atlas else {
        return;
    };
    let table = layers.0.read().unwrap();
    // wait until every defined tile has either reported its layer or failed
    for handle in atlas.slots.iter().flatten() {
        let reported = handle.path().is_some_and(|path| table.contains_key(path));
        let settled = matches!(
            asset_server.get_load_state(handle),
            Some(LoadState::Loaded | LoadState::Failed(_))
        );
        if !reported && !settled {
            return;
        }
    }

    // `graphics.texture_detail`: every layer starts `skip` levels down its
    // chain, a slice off the front of each layer's bytes (never below one
    // 4x4 block)
    let skip = texture_detail
        .map_or(0, |level| level.skipped_mips())
        .min(TILE_LAYER_MIPS - 3);
    let skip_bytes = crate::assets::texture_detail::bc1_top_levels_len(TILE_LAYER_SIZE, skip);
    let white = white_tile_layer();
    let mut missing = 0usize;
    let mut counts = [0usize; TILE_ARRAY_COUNT];
    let arrays = std::array::from_fn(|array| {
        let first = array * TILE_ARRAY_LAYERS as usize;
        let ids = &atlas.slots[first..first + TILE_ARRAY_LAYERS as usize];
        // as many layers as the highest defined id in this range needs, and
        // never exactly one: wgpu's GL backend picks a texture's GL type from
        // its layer count, and a single layer becomes a plain 2D texture that
        // cannot be sampled as the array the shader declares
        let count = ids
            .iter()
            .rposition(Option::is_some)
            .map_or(1, |i| i + 1)
            .max(2);
        counts[array] = count;
        let mut data = Vec::with_capacity(count * (tile_layer_len() - skip_bytes));
        for slot in &ids[..count] {
            let layer = slot
                .as_ref()
                .and_then(Handle::path)
                .and_then(|path| table.get(path));
            match layer {
                Some(layer) => data.extend_from_slice(&layer[skip_bytes..]),
                None => {
                    missing += slot.is_some() as usize;
                    data.extend_from_slice(&white[skip_bytes..]);
                }
            }
        }
        images.add(tile_array_image(count as u32, skip, data))
    });
    drop(table);
    if missing > 0 {
        warn!("{missing} ground tiles have no texture-array layer and render white");
    }
    // Trimming each array to its highest defined id leaves the rest of the id
    // space addressable but unbacked, and nothing downstream notices: the splat
    // shader derives the layer from the raw 10-bit id alone
    // (`terrain_splat.wgsl`, `let layer = i32(tile & 255u)`), and WGSL leaves an
    // out-of-range array layer indeterminate. Whether any map vertex names such
    // an id is a property of the drop, so say what is missing and let the log
    // answer it, rather than assuming either way.
    let defined = atlas.slots.iter().flatten().count();
    if let Some(first) = first_unbacked_id(&counts) {
        warn!(
            "ground tile arrays: {} of {} tile ids have no layer, first {first} \
             (tile2d.ifo defines {defined}); the splat shader addresses every id, and WGSL \
             leaves an out-of-range array layer indeterminate",
            unbacked_layers(&counts),
            TILE_ARRAY_COUNT * TILE_ARRAY_LAYERS as usize,
        );
    }
    info!("ground tile arrays built: {defined} tile ids over {counts:?} layers");
    layers.0.write().unwrap().clear();
    commands.insert_resource(TerrainTileArrays(arrays));
}

/// Layers the splat shader can address but no array holds: per array, everything
/// from its built layer count up to [`TILE_ARRAY_LAYERS`].
fn unbacked_layers(counts: &[usize; TILE_ARRAY_COUNT]) -> usize {
    counts
        .iter()
        .map(|count| (TILE_ARRAY_LAYERS as usize).saturating_sub(*count))
        .sum()
}

/// The lowest tile id with no layer behind it, or `None` when every id has one.
fn first_unbacked_id(counts: &[usize; TILE_ARRAY_COUNT]) -> Option<u32> {
    counts
        .iter()
        .position(|count| *count < TILE_ARRAY_LAYERS as usize)
        .map(|array| array as u32 * TILE_ARRAY_LAYERS + counts[array] as u32)
}

/// One tile array of `layers` layers, each starting `skip` levels down the
/// normalized chain.
fn tile_array_image(layers: u32, skip: u32, data: Vec<u8>) -> Image {
    let size = TILE_LAYER_SIZE >> skip;
    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        TextureFormat::Bc1RgbaUnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = TILE_LAYER_MIPS - skip;
    // a single-layer array must still be viewed as an array
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.data = Some(data);
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A drop that defines every id leaves nothing unbacked — the log must stay
    /// quiet there, which is the only reason the warning is trustworthy when it
    /// does fire.
    #[test]
    fn a_complete_id_space_is_fully_backed() {
        let counts = [TILE_ARRAY_LAYERS as usize; TILE_ARRAY_COUNT];
        assert_eq!(unbacked_layers(&counts), 0);
        assert_eq!(first_unbacked_id(&counts), None);
    }

    /// The layer counts a 603-id `tile2d.ifo` produces (dense 0..602, measured
    /// 2026-10-14): arrays 0 and 1 full, array 2 trimmed to 91, array 3 at its
    /// two-layer floor. The first unbacked id must come out as 603 — the first
    /// id that file does not define, and the map does reference ids above it.
    #[test]
    fn a_603_id_drop_leaves_the_rest_of_the_space_unbacked() {
        let counts = [256, 256, 91, 2];
        assert_eq!(first_unbacked_id(&counts), Some(603));
        assert_eq!(unbacked_layers(&counts), 419);
    }

    /// A gap in the *first* array is reported from there, not from the last one.
    #[test]
    fn the_lowest_gap_wins() {
        let counts = [100, 256, 256, 256];
        assert_eq!(first_unbacked_id(&counts), Some(100));
        assert_eq!(unbacked_layers(&counts), 156);
    }

    /// The two-layer floor `build_tile_arrays` applies to an empty array is
    /// itself a gap, and at the right id.
    #[test]
    fn the_two_layer_floor_of_an_empty_array_is_a_gap() {
        let counts = [256, 256, 256, 2];
        assert_eq!(first_unbacked_id(&counts), Some(768 + 2));
        assert_eq!(unbacked_layers(&counts), 254);
    }
}
