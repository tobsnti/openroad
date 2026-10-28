// https://github.com/DummkopfOfHachtenduden/SilkroadDoc/wiki/JMXVBMT

use std::cmp::Reverse;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bevy::asset::{io::Reader, Asset, AssetLoader, AssetServer, LoadContext};
use bevy::color::LinearRgba;
use bevy::pbr::ExtendedMaterial;
use bevy::prelude::{trace, warn, AlphaMode, Color, Handle, Image, StandardMaterial};
use bevy::reflect::TypePath;
use bevy::render::render_resource::Face;
use bevy::utils::default;
use bytes::Buf;
use thiserror::Error;

use crate::assets::bmt::rim::{RimExtension, RimSettings, SroRimMaterial};
use crate::assets::bmt::sheen::{
    SheenExtension, SheenSettings, SroSheenMaterial, ENV_SPHEREMAP_PATH, SHEEN_ALPHA_CUTOUT,
    SHINE_SPHEREMAP_PATH,
};

use crate::util::buf_ext::BufExt;

#[derive(Asset, TypePath, Debug, Default, Clone)]
#[allow(dead_code)]
pub struct SroMaterial {
    pub name: String,
    pub path: PathBuf,
    pub ambient: Color,
    pub diffuse: Color,
    pub specular: Color,
    pub emissive: Color,
    pub power: f32,
    pub flag: u32,
    pub diffuse_map_path: PathBuf,
    pub diffuse_map_float: f32, // could be lightness? contains at least same value as in Color
    pub diffuse_map_unk0: u8,
    pub diffuse_map_unk1: u8,
    /// `1` = the path above is the **full archive path** (`prim\mtrl\…`),
    /// `0` = a bare file name next to the `.bmt` itself.
    ///
    /// Named `diffuse_map_is_relative` until 2026-10-26, which said the
    /// opposite of what it holds. Re-counted 2026-10-28 over the whole corpus
    /// below: the byte is `1` on **729** materials, **all 729** naming an
    /// archive-rooted path (726 under `prim\`, 3 under `compound\`), and `0`
    /// on **15,497**, **none** of which carry a separator (426 of those are
    /// empty). The behaviour was always right — only the name was backwards,
    /// which is why `bsr2glb/bundle.rs` had to explain it against itself.
    /// JMX-File-Editor and SroPk2Studio (both MIT) call the field `isAbsolute`.
    pub diffuse_map_is_absolute: bool,
    /// The normal map of the extra block a material can carry, as the file
    /// spells it; `None` when there is no block, which is the usual case even
    /// among flagged materials (see [`JMXVBMT::from`]). Eight blocks in three
    /// files of the user's data, read so that those files come out whole —
    /// nothing samples the map yet.
    pub normal_map_path: Option<PathBuf>,
}

/// Fixed part of one material record: four RGBA colors, power, flag, the
/// diffuse float and its three trailing bytes — everything but the two
/// length-prefixed strings.
const MATERIAL_FIXED_LEN: usize = 4 * 16 + 4 + 4 + 4 + 3;

/// A length-prefixed string, refusing a prefix the buffer cannot hold.
///
/// `get_fixed_size_string` slices `chunk()` directly, so a bogus `u32` length
/// **panics** rather than failing the load. Both readings in [`JMXVBMT::from`]
/// run into exactly that on the user's own data — the plain one on **2** files,
/// the normal-map one on **30** — and a reading that has to be rejected must
/// come back, not abort. Same guard the `.bms` header reader grew for the same
/// reason.
fn read_string<T: Buf>(value: &mut T) -> Option<String> {
    if value.remaining() < 4 {
        return None;
    }
    let len = u32::from_le_bytes(value.chunk().get(..4)?.try_into().ok()?) as usize;
    if value.remaining() < 4 + len {
        return None;
    }
    Some(value.get_double_len_string())
}

/// A length-prefixed path with SRO's backslashes rewritten to the host's
/// separator.
fn read_path<T: Buf>(value: &mut T) -> Option<PathBuf> {
    Some(PathBuf::from(
        read_string(value)?
            .replace(r"\\", r"\")
            .replace(r"\", std::path::MAIN_SEPARATOR_STR),
    ))
}

impl SroMaterial {
    /// Reads one material, or `None` when the buffer does not hold a whole one.
    ///
    /// `normal_map_block` is the array shape being tried: with it, a material
    /// whose flag carries `0x2000` is followed by one more path and four bytes
    /// — see [`JMXVBMT::from`], which decides per file which shape fits.
    fn read<T: Buf>(value: &mut T, normal_map_block: bool) -> Option<Self> {
        let name = read_string(value)?;
        if value.remaining() < 4 * 16 + 4 + 4 + 4 {
            return None;
        }
        let diffuse = value.get_color_rgba();
        let ambient = value.get_color_rgba();
        let specular = value.get_color_rgba();
        let emissive = value.get_color_rgba();
        let power = value.get_f32_le();
        let flag = value.get_u32_le();
        let diffuse_map_path = read_path(value)?;
        if value.remaining() < 4 + 3 {
            return None;
        }
        let mut material = Self {
            name,
            diffuse,
            ambient,
            specular,
            emissive,
            power,
            flag,
            diffuse_map_path,
            diffuse_map_float: value.get_f32_le(),
            diffuse_map_unk0: value.get_u8(),
            diffuse_map_unk1: value.get_u8(),
            diffuse_map_is_absolute: value.get_u8() == 1,
            ..default()
        };
        if normal_map_block && material.has_bump_map() {
            material.normal_map_path = Some(read_path(value)?);
            if value.remaining() < 4 {
                return None;
            }
            // four bytes nobody has explained yet; SroPk2Studio (MIT) reads
            // them as one u32 and calls them `normalMapUnk`
            value.advance(4);
        }
        Some(material)
    }
}

// PrimMtrlFlag, from a census of the user's own extracted `Data.pk2`
// (2026-10-28): 16,226 materials in all 4,171 of its `.bmt` files, every one
// signature `JMXVBMT 0102` and every one read to its last byte by the reader
// below. Only **7** of the 32 bits are ever set, in **19** distinct flag words
// — that is the number worth keeping, because it is what says the other 25
// bits need no implementation:
//   0x0001 bit0  two-sided / no backface cull       4,439  27.4%  is_two_sided
//   0x0004 bit2  D3D sun specular enable              192   1.2%  is_specular_emphasis
//   0x0008 bit3  self-illuminated / emissive          158   1.0%  is_emissive
//   0x0040 bit6  ALWAYS set - mandatory marker     16,226 100.0%  has_color_tint (misnomer)
//   0x0100 bit8  has diffuse map / texture         15,748  97.1%  has_diffuse_map
//   0x0200 bit9  has alpha channel                  6,066  37.4%  has_alpha_channel
//   0x2000 bit13 the wiki's "BumpMap"                 121   0.7%  has_bump_map
// Bits 1, 4, 5, 7, 10-12 and 14-31 are set nowhere.
//
// Two earlier runs of the same census stood here, at 12,227 materials
// (2026-07-09) and 14,485 in 3,757 of 3,759 files (2026-10-26). Both
// enumerated only the lowercase spelling: **412** of the 4,171 files are
// spelled `.BMT`, and two of those are among the three `JMXVBMT::from` has to
// read the second way. A census carries its date and its population for
// exactly that reason — a narrower one reads as a fact about the format.

#[allow(dead_code)]
impl SroMaterial {
    /// PrimMtrlFlag Bit6 (0x40). The wiki labels this "ColorTint?", but the
    /// census found it set on 100% of materials - it is a mandatory
    /// "lit material" format marker, so this predicate is always true.
    pub fn has_color_tint(&self) -> bool {
        self.flag & 0x40 != 0
    }

    pub fn has_diffuse_map(&self) -> bool {
        self.flag & 0x100 != 0
    }

    pub fn has_alpha_channel(&self) -> bool {
        self.flag & 0x200 != 0
    }

    /// PrimMtrlFlag Bit13 (0x2000), the wiki's "BumpMap": **121 materials in
    /// 34 files** carry it (census above), so it is live, not dead.
    ///
    /// What it does **not** do is announce the normal-map block: only **3** of
    /// those 34 files carry one. `mob/arabia/khulood.bmt` and
    /// `mob/bossdungeon/khulood_boss_dungeon.BMT` hold the same two material
    /// names under the same two flag words (`0x2344`, `0x2345`) and disagree
    /// about the block, so no flag can be the switch — [`JMXVBMT::from`]
    /// decides per file instead.
    pub fn has_bump_map(&self) -> bool {
        self.flag & 0x2000 != 0
    }

    /// PrimMtrlFlag Bit1 (0x1): render both faces (no backface cull).
    /// Set on hair, foliage/leaves, grass, flags, ropes, caps, and paper-ward
    /// talismans - thin card geometry authored as single-sided quads. Verified
    /// against Data.pk2: correlates exactly with two-sided geometry and is
    /// independent of the alpha bit (0x200).
    pub fn is_two_sided(&self) -> bool {
        self.flag & 0x1 != 0
    }

    /// PrimMtrlFlag Bit3 (0x8): self-illuminated / emissive. Set almost
    /// exclusively on building materials whose textures are named `*_winlight`,
    /// `window`, `light`, `jewel`, xmas lights - the panes/lamps that glow at
    /// night. The flag (not the emissive color field) is the render signal and
    /// drives `StandardMaterial::emissive`, always-on until a day/night cycle
    /// exists. Verified against Data.pk2 (104 materials, 92 of them buildings).
    pub fn is_emissive(&self) -> bool {
        self.flag & 0x8 != 0
    }

    /// PrimMtrlFlag Bit2 (0x4): classic D3D sun specular. Verified in
    /// sro_client.exe (material apply fn ~0xc82300): the flag turns on
    /// `D3DRS_SPECULARENABLE` for the draw (restore fn ~0xc82450 turns it
    /// back off) and the BMT specular color + power fill the D3DMATERIAL9,
    /// so the directional sun produces a fixed-function Phong highlight.
    /// Only 19 materials in all of Data.pk2 opt in (Jangan palace set,
    /// stone lion statues, horse skulls, ... - power 7-170, tinted colors
    /// like (0.5,0.25,0)); everything else stays pure diffuse.
    pub fn is_specular_emphasis(&self) -> bool {
        self.flag & 0x4 != 0
    }
}

#[derive(TypePath, Asset, Debug, Clone)]
pub struct JMXVBMT {
    pub materials: Vec<SroMaterial>,
    /// Each material's diffuse texture, keyed by [`material_label`]. Filled by
    /// [`BmtLoader`] (empty when parsed standalone); materials with no diffuse
    /// map have no entry. Kept so the rim/sheen variants can be built on
    /// demand from the loaded set instead of up front for every material.
    pub diffuse_textures: HashMap<String, Handle<Image>>,
}

/// One reading of the material array: what came out, and how much of the file
/// it left unexplained.
struct Reading {
    materials: Vec<SroMaterial>,
    /// Bytes between the last material read and the end of the file.
    leftover: usize,
    /// Every material the count claims came out.
    complete: bool,
}

impl Reading {
    /// This reading explains the whole file.
    fn exact(&self) -> bool {
        self.complete && self.leftover == 0
    }

    /// Complete beats cut short, then the smaller tail wins.
    fn rank(&self) -> (bool, Reverse<usize>) {
        (self.complete, Reverse(self.leftover))
    }
}

/// Reads the material array in one of its two shapes (see [`JMXVBMT::from`]).
fn read_array(body: &[u8], count: u32, normal_map_block: bool) -> Reading {
    let mut cursor = Cursor::new(body);
    // a count is one u32 of a possibly broken file: only trust it as far
    // as the bytes that are actually there
    let capacity = count.min((cursor.remaining() / MATERIAL_FIXED_LEN) as u32 + 1);
    let mut materials = Vec::with_capacity(capacity as usize);
    while materials.len() < count as usize {
        let Some(material) = SroMaterial::read(&mut cursor, normal_map_block) else {
            break;
        };
        materials.push(material);
    }
    Reading {
        complete: materials.len() == count as usize,
        leftover: cursor.remaining(),
        materials,
    }
}

impl JMXVBMT {
    /// Reads a material set from `body` — the file after its 12-byte
    /// signature, through to its last byte.
    ///
    /// Idea: the material array has two shapes and nothing in the file says
    /// which one a file is in, so read it both ways and keep the reading that
    /// explains the file — the one that lands on its last byte. The second
    /// shape puts one more path (a normal map) plus four bytes after every
    /// material whose flag carries `0x2000`. The wiki and SroPk2Studio's
    /// `Bmt.h` (MIT) read that flag as announcing the block; measured over the
    /// census above it does not, because **30** of the 34 flagged files
    /// consume exactly *without* it, **3** only *with* it, and none comes out
    /// even both ways. The shape is decidable — per file, not per flag.
    ///
    /// Those three cost real resources, each named by a `.bsr` of the same
    /// install: without the block `item/etc/avatar_w_ghostcaptain_2.bmt` ends
    /// after 1 of its 5 materials and `mob/bossdungeon/khulood_boss_dungeon
    /// .BMT` after 1 of 2 (815 and 237 bytes left). The third,
    /// `mustafa_boss_dungeon_stone.BMT`, is why the leftover decides and not
    /// the material count: it reads *complete* with 122 bytes left, its second
    /// material the first one's normal-map path taken for a name. And one file
    /// is exact under neither shape — `mob/arabia/mustafa.bmt` leaves two
    /// bytes without the block and is cut short with it, so the smaller tail
    /// keeps it; those two bytes are a file tail, not a material field.
    ///
    /// SroPk2Studio's `Bmt.cpp` (MIT) reads the array twice for the same
    /// reason, after the flag desynced it on the mobile remake's
    /// `karkadann.bmt`; its header claims the block "does not exist at all in
    /// VSRO 1.188 files", which the three files above refute for this build.
    /// The method is adopted, that claim is not, and no code was ported.
    pub fn from(body: &[u8], path: PathBuf) -> Self {
        let Some((count, body)) = body.split_at_checked(4) else {
            warn!("{}: no material count", path.display());
            return Self {
                materials: Vec::new(),
                diffuse_textures: HashMap::new(),
            };
        };
        let count = u32::from_le_bytes([count[0], count[1], count[2], count[3]]);

        let plain = read_array(body, count, false);
        // Only a file that does not come out even is read a second time: an
        // exact reading is already the best either shape can do.
        let reading = if plain.exact() {
            plain
        } else {
            let bumped = read_array(body, count, true);
            if bumped.rank() > plain.rank() {
                bumped
            } else {
                plain
            }
        };
        if !reading.complete {
            warn!(
                "{}: stops after {} of {count} materials - neither shape of the array fits \
                 what is left",
                path.display(),
                reading.materials.len()
            );
        }

        let mut materials = reading.materials;
        for material in &mut materials {
            material.path = path.clone();
        }
        Self {
            materials,
            diffuse_textures: HashMap::new(),
        }
    }

    /// The material `name` refers to (case-insensitively, see
    /// [`material_label`]) and its diffuse texture, if both exist.
    pub fn material_with_texture(&self, name: &str) -> Option<(&SroMaterial, &Handle<Image>)> {
        let label = material_label(name);
        let texture = self.diffuse_textures.get(&label)?;
        let material = self
            .materials
            .iter()
            .find(|mat| material_label(&mat.name) == label)?;
        Some((material, texture))
    }
}

/// The config-derived settings the rim and sheen material variants start from
/// (see [`SroMaterial::to_rim_material`] / [`SroMaterial::to_sheen_material`]).
/// The client bin inserts it from `graphics.{sheen,rim}` before the app runs;
/// without it (tools, tests) the variants fall back to the faithful defaults
/// (no ambient rim, neutral sheen).
#[derive(bevy::prelude::Resource, Clone, Copy, Debug)]
pub struct BmtMaterialDefaults {
    /// Base settings of the sheen / sheen-cutout variants.
    pub sheen: SheenSettings,
    /// `Some` = character-class meshes use a rim variant with these settings
    /// (see `PreparedMeshGroups::prepare`); `None` = rim disabled.
    pub rim: Option<RimSettings>,
    /// `false` = sheen resources draw with the plain opaque material
    /// ([`SroMaterial::to_unsheened_material`]) instead of the sheen extension
    /// (`graphics.sheen.enabled`, the Metallic Sheen option).
    pub sheen_enabled: bool,
}

impl Default for BmtMaterialDefaults {
    fn default() -> Self {
        Self {
            sheen: SheenSettings::default(),
            rim: None,
            sheen_enabled: true,
        }
    }
}

/// Whether cut-out materials spread their edge across the MSAA samples instead
/// of deciding each texel yes/no, shared with the `.bmt` loader.
///
/// Same mechanism and the same reason as [`TextureDetailLevel`]: the loader
/// runs on the async task pool and this module is part of the parser library,
/// which cannot see config types. The client pushes
/// `graphics.foliage_alpha_to_coverage` in here; materials keep the mode they
/// were loaded with, so a change reaches what loads afterwards.
///
/// [`TextureDetailLevel`]: crate::assets::texture_detail::TextureDetailLevel
#[derive(bevy::prelude::Resource, Clone, Default)]
pub struct CutoutAlphaToCoverage(pub Arc<AtomicBool>);

impl CutoutAlphaToCoverage {
    pub fn enabled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn set(&self, enabled: bool) {
        self.0.store(enabled, Ordering::Relaxed);
    }
}

/// The alpha mode of a cut-out: the original's hard test at `cutoff`, or the
/// same edge dithered across the MSAA samples.
///
/// Idea: `Mask` turns every leaf edge into one yes/no decision per pixel, so
/// the edge crawls as the camera turns and FXAA can only blur it afterwards.
/// `AlphaToCoverage` hands the same alpha to the MSAA coverage mask, which
/// `high`/`ultra` already pay for — a deliberate deviation from the original's
/// alpha test (ref 0x80, `D3DCMP_GREATEREQUAL`, see
/// [`SroMaterial::to_standard_material`]) and therefore a switch.
///
/// Measured in `bevy_pbr-0.20.0`, so the fallback costs nothing and the
/// sorting does not change:
/// * at `Msaa::Off` the pipeline key falls back to `MAY_DISCARD`
///   (`src/material.rs:697-700`) and the shader then discards at
///   `alpha_cutoff`, which stays 0.5 for this mode (`src/pbr_material.rs:1190`,
///   `src/render/pbr_functions.wesl:110-118`) — identical to `Mask(0.5)`;
/// * the phase is unchanged, A2C sorts with `Opaque`
///   (`src/material.rs:1308,1754`).
pub fn cutout_alpha_mode(cutoff: f32, alpha_to_coverage: bool) -> AlphaMode {
    if alpha_to_coverage {
        AlphaMode::AlphaToCoverage
    } else {
        AlphaMode::Mask(cutoff)
    }
}

/// The cut-out threshold every `.bmt` material with an alpha channel gets: the
/// original's alpha test, `ref 0x80` with `D3DCMP_GREATEREQUAL`, in bevy's
/// 0..1 scale. Named because the live switch below has to recognise a cut-out
/// again after [`AlphaMode::AlphaToCoverage`] has thrown the number away.
pub const CUTOUT_ALPHA_CUTOFF: f32 = 0.5;

/// Move the **already loaded** cut-out materials to `alpha_to_coverage`, and
/// report how many changed.
///
/// Idea: [`CutoutAlphaToCoverage`] only reaches materials that are *built*
/// after it changes, so a live A/B — one process, one scene, one frame apart —
/// needs the loaded ones rewritten too. That is the same move
/// `render_debug`'s `effect_ldr_additive` makes for effect materials, and this
/// is its cut-out counterpart; the switch lives in `dev/render_debug.rs`, the
/// rule stays here next to the one that built those materials.
///
/// The pass is exactly the inverse of [`cutout_alpha_mode`]: a material moves
/// between `Mask(CUTOUT_ALPHA_CUTOFF)` and `AlphaToCoverage` and nothing else
/// is touched. Every other threshold therefore stays where it is — in
/// particular the sheen cut-outs at [`SHEEN_ALPHA_CUTOUT`] (1/255), whose
/// alpha is a *sheen mask* rather than transparency, which is the exclusion
/// the build-time path makes too.
pub fn retarget_cutout_materials(
    materials: &mut bevy::asset::Assets<StandardMaterial>,
    alpha_to_coverage: bool,
) -> usize {
    let want = cutout_alpha_mode(CUTOUT_ALPHA_CUTOFF, alpha_to_coverage);
    // ids first: `Assets::iter` borrows the whole collection, and the rewrite
    // below needs it mutably (and marks each touched asset modified, which is
    // what re-prepares its pipeline).
    let cutouts: Vec<_> = materials
        .iter()
        .filter(|(_, material)| is_cutout(material.alpha_mode))
        .map(|(id, _)| id)
        .collect();
    let mut changed = 0;
    for id in cutouts {
        if let Some(mut material) = materials.get_mut(id) {
            if material.alpha_mode != want {
                material.alpha_mode = want;
                changed += 1;
            }
        }
    }
    changed
}

/// Whether `mode` is one of the two a cut-out can be in.
fn is_cutout(mode: AlphaMode) -> bool {
    mode == AlphaMode::AlphaToCoverage || mode == AlphaMode::Mask(CUTOUT_ALPHA_CUTOFF)
}

/// Loads a `.bmt` material set. Only the default (alpha-masked) variant of each
/// material becomes a labeled sub-asset; the rim and sheen variants are built
/// on demand by the spawn path (`SroMaterialVariants` in the client), because
/// building all of them for every material of every set meant several extra
/// material assets per material — each extracted and prepared by the renderer
/// — when a given mesh only ever uses one.
#[derive(bevy::reflect::TypePath)]
pub struct BmtLoader {
    cutouts: CutoutAlphaToCoverage,
}

impl bevy::prelude::FromWorld for BmtLoader {
    fn from_world(world: &mut bevy::prelude::World) -> Self {
        world.init_resource::<CutoutAlphaToCoverage>();
        Self {
            cutouts: world.resource::<CutoutAlphaToCoverage>().clone(),
        }
    }
}

#[derive(Error, Debug)]
pub enum BmtLoaderError {
    #[error("IO error: {0}")]
    IO(std::io::Error),
    #[error("invalid signature: {0}")]
    Signature(String),
}

// type MaterialSet = Vec<SroMaterial>;

impl AssetLoader for BmtLoader {
    type Asset = JMXVBMT;
    type Settings = ();
    type Error = BmtLoaderError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut buf = Vec::new();
        reader
            .read_to_end(&mut buf)
            .await
            .map_err(|e| BmtLoaderError::IO(e))?;
        let sig = Cursor::new(&buf).get_fixed_size_string(12);
        if sig != "JMXVBMT 0102" {
            return Err(BmtLoaderError::Signature(sig));
        }
        let mut path = load_context.path().path().to_path_buf();
        path.pop();
        // the signature matched in full, so the 12 bytes are there; the reader
        // needs the rest as a slice because it reads it twice
        let mut material = JMXVBMT::from(&buf[12..], path);

        let mut diffuse_textures = HashMap::new();
        for mat in &material.materials {
            // info!("material: {}", &mat.name);
            let image: Handle<Image> = if mat.diffuse_map_is_absolute {
                let path = format!("data://{}", mat.diffuse_map_path.display());
                // info!("relative path: {}", path);
                // PathBuf::from("")
                load_context.load(path)
            } else {
                let path = format!("data://{}", mat.path.display());
                // info!("absolute path part1: '{}'", path);
                let path = format!("{}/{}", path, mat.diffuse_map_path.display());
                // info!("absolute path part2: '{}'", mat.diffuse_map_path.display());
                let p = format!("{}", mat.diffuse_map_path.display());
                if p.is_empty() {
                    trace!(
                        "empty material path in: {} for material {}",
                        load_context.path().path().display(),
                        &mat.name
                    );
                    continue;
                }
                // let mut path = PathBuf::from(format!("data://{}/{}", mat.path.display(), mat.diffuse_map_path.display()));
                // path.extend(&mat.diffuse_map_path);
                // info!("absolute path: {}", &path);
                load_context.load(path)
            };
            // let image = load_context.load_direct(path).await?;
            // let image: Handle<Image> = load_context.load(path);
            let standard_material = mat.to_masked_material(image.clone(), self.cutouts.enabled());
            load_context.add_labeled_asset(material_label(&mat.name), standard_material);
            diffuse_textures.insert(material_label(&mat.name), image);
        }
        material.diffuse_textures = diffuse_textures;

        Ok(material)
    }

    fn extensions(&self) -> &[&str] {
        &["bmt"]
    }
}

/// The two sphere maps every sheen material samples, `(shine, env)`, loaded as
/// non-color intensity maps (see [`DdjSettings::non_color`]): their stored
/// bytes are the reflection intensity, so an sRGB view would decode mid-gray
/// 128 to 0.216 instead of the intended 0.5 and halve the whole chrome term.
///
/// [`DdjSettings::non_color`]: crate::assets::ddj::DdjSettings
pub fn sheen_probe_textures(asset_server: &AssetServer) -> (Handle<Image>, Handle<Image>) {
    let load = |path: &'static str| -> Handle<Image> {
        asset_server
            .load_builder()
            .with_settings(|settings: &mut crate::assets::ddj::DdjSettings| {
                settings.non_color = true
            })
            .load(path)
    };
    (load(SHINE_SPHEREMAP_PATH), load(ENV_SPHEREMAP_PATH))
}

impl SroMaterial {
    /// The always-on rim variant for character-class meshes: the default
    /// (Masked) material plus the config ambient rim.
    pub fn to_rim_material(&self, texture: Handle<Image>, settings: RimSettings) -> SroRimMaterial {
        ExtendedMaterial {
            base: self.to_standard_material(Some(texture), MaterialVariant::Masked),
            extension: RimExtension { settings },
        }
    }

    /// The sheen variant, for resources whose texture alpha is a sheen mask
    /// instead of transparency (see `SroResource::alpha_is_sheen`): an extended
    /// material whose fragment shader turns that alpha into per-texel metallic
    /// (sheen.rs). `probes` is [`sheen_probe_textures`]: the enhancement
    /// highlight map (sampled only once a shine tint is set) and the base
    /// chrome probe (always sampled).
    ///
    /// `cutout` adds the original's alpha test (GREATEREQUAL ref 1), for
    /// resources whose EnvMap mod flags exact-zero alpha as cutout (e.g. glaive
    /// blade shapes punched out of the atlas). Its base must be Mask, not
    /// Opaque: cameras run a depth prepass and the extension only replaces the
    /// main-pass fragment, so an Opaque base writes prepass/shadow depth for
    /// the very texels the main pass discards — geometry behind can't render
    /// there and the cutouts turn into opaque black holes (deg-7/8 CH shields).
    /// Mask makes the stock prepass/shadow shaders discard the same texels,
    /// while the main pass still receives the raw sampled alpha as the sheen
    /// mask (pbr_input_from_standard_material applies no alpha_discard).
    /// What a sheen resource draws with when metallic sheen is off: the same
    /// opaque base the sheen variant extends, without the extension. Its
    /// texture alpha is a sheen mask, not transparency, so it must not be the
    /// default Masked material, which would punch it out. `cutout` keeps the
    /// original's exact-zero alpha test, as in [`Self::to_sheen_material`].
    pub fn to_unsheened_material(&self, texture: Handle<Image>, cutout: bool) -> StandardMaterial {
        let mut base = self.to_standard_material(Some(texture), MaterialVariant::Opaque);
        if cutout {
            base.alpha_mode = AlphaMode::Mask(SHEEN_ALPHA_CUTOUT);
        }
        base
    }

    pub fn to_sheen_material(
        &self,
        texture: Handle<Image>,
        settings: SheenSettings,
        cutout: bool,
        (shine_texture, env_texture): (Handle<Image>, Handle<Image>),
    ) -> SroSheenMaterial {
        let mut base = self.to_standard_material(Some(texture), MaterialVariant::Opaque);
        let settings = if cutout {
            base.alpha_mode = AlphaMode::Mask(SHEEN_ALPHA_CUTOUT);
            SheenSettings {
                alpha_cutout: SHEEN_ALPHA_CUTOUT,
                ..settings
            }
        } else {
            settings
        };
        ExtendedMaterial {
            base,
            extension: SheenExtension {
                settings,
                shine_texture,
                env_texture,
            },
        }
    }
}

/// Canonical label of a material's default variant within its .bmt. SRO
/// material names are case-insensitive: a mesh (`.bms`) may reference `Gyo`
/// while the `.bmt` stores it as `gyo` (or `waterghost` vs `WaterGhost`). Bevy's
/// labeled-asset lookup is case-*sensitive*, so both the producing side (the
/// loader below) and the requesting side (mesh assembly in `commands`) route the
/// name through this to lowercase it, making the labels match either way.
pub fn material_label(material_name: &str) -> String {
    material_name.to_ascii_lowercase()
}

/// Key of a material's always-on-rim variant (the Masked base plus the config
/// ambient rim; only used when [`BmtMaterialDefaults::rim`] is `Some`). Not a
/// labeled sub-asset: the variant is built on demand (`SroMaterialVariants`).
pub fn rim_material_label(material_name: &str) -> String {
    format!("{}.rim", material_label(material_name))
}

/// Key of a material's metallic-sheen variant (built on demand, like the rim).
pub fn sheen_material_label(material_name: &str) -> String {
    format!("{}.sheen", material_label(material_name))
}

/// Key of the sheen variant that also cuts out exact-zero alpha texels
/// (for resources with the EnvMap alpha-test flag,
/// [`SroResource::sheen_alpha_test`](crate::assets::bsr::resource::SroResource::sheen_alpha_test)).
pub fn sheen_cutout_material_label(material_name: &str) -> String {
    format!("{}.sheen_cutout", material_label(material_name))
}

/// Emissive intensity of self-illuminated (flag 0x8) materials; visual
/// calibration constant (bevy's emissive is tonemapped scene-linear, so
/// plain 1.0 washes out next to daylight).
const EMISSIVE_BOOST: f32 = 2.0;

/// Reflectance of flag-0x4 (sun specular) materials; visual calibration
/// constant (0.5 = the 4% dielectric F0 PBR default, matching SheenSettings).
const SPECULAR_EMPHASIS_REFLECTANCE: f32 = 0.5;

/// D3D fixed-function Phong exponent -> perceptual roughness via the Phong ->
/// Beckmann width conversion alpha = sqrt(2 / (power + 2)). The census range
/// (power 7..170) maps to ~0.47..0.11. The 0.1 floor stays above bevy's
/// internal roughness minimum and avoids needle-thin aliasing highlights; the
/// 0.8 ceiling equals the pure-diffuse default.
fn phong_power_to_perceptual_roughness(power: f32) -> f32 {
    (2.0 / (power + 2.0)).sqrt().clamp(0.1, 0.8)
}

/// Which [`StandardMaterial`] base of a material to build. The sheen variant
/// ([`SroMaterial::to_sheen_material`]) is an [`SroSheenMaterial`]
/// (extended material) wrapping the Opaque base, whose fragment shader
/// turns the texture alpha into per-texel metallic — see `sheen.rs`.
#[derive(Clone, Copy, PartialEq)]
pub enum MaterialVariant {
    /// Default: texture alpha is transparency (alpha test).
    Masked,
    /// Texture alpha is data (env/sheen mask), render fully opaque
    /// (see `SroResource::alpha_is_sheen`).
    Opaque,
}

impl SroMaterial {
    /// The loader's default variant, with the cut-out edge dithered across the
    /// MSAA samples when [`CutoutAlphaToCoverage`] is on.
    ///
    /// Only a material that *is* a cut-out changes: the cutoff stays this
    /// type's own ([`Self::to_standard_material`]), and an opaque material
    /// stays opaque — alpha-to-coverage on a material whose alpha means
    /// nothing would punch holes by whatever the texture happens to carry.
    /// That is also why the sheen variants keep their `Mask`: there the alpha
    /// channel is a *sheen mask*, not transparency (`SroResource::alpha_is_sheen`),
    /// so coverage would read shine as see-through metal.
    pub fn to_masked_material(
        &self,
        texture: Handle<Image>,
        alpha_to_coverage: bool,
    ) -> StandardMaterial {
        let mut material = self.to_standard_material(Some(texture), MaterialVariant::Masked);
        if let AlphaMode::Mask(cutoff) = material.alpha_mode {
            material.alpha_mode = cutout_alpha_mode(cutoff, alpha_to_coverage);
        }
        material
    }

    /// Builds one rendering variant of this material; the .bmt loader adds
    /// the Masked one as a labeled sub-asset, the rim/sheen variants wrap
    /// these on demand, and resources pick per their alpha semantics.
    pub fn to_standard_material(
        &self,
        texture: Option<Handle<Image>>,
        variant: MaterialVariant,
    ) -> StandardMaterial {
        match texture {
            Some(texture) => {
                let mut material = StandardMaterial::from(texture);
                material.alpha_mode =
                    if self.has_alpha_channel() && variant == MaterialVariant::Masked {
                        // the original engine's alpha test uses ref 0x80 with
                        // D3DCMP_GREATEREQUAL (seen in leaf Material ModData)
                        AlphaMode::Mask(CUTOUT_ALPHA_CUTOFF)
                    } else {
                        AlphaMode::Opaque
                    };
                // material.unlit = true;
                // The original engine renders non-EnvMap, non-flag-0x4
                // materials purely diffuse (fixed-function, no specular);
                // only the sheen variant's masked texels turn specular
                // (sro_sheen.wesl mixes toward its own reflectance by the
                // mask). Any base reflectance makes skin/cloth catch
                // env-map highlights.
                material.reflectance = 0.0;
                material.metallic = 0.0;
                material.perceptual_roughness = 0.8;
                if self.is_specular_emphasis() && self.power > 0.0 {
                    // Flag 0x4 is the original's per-draw
                    // D3DRS_SPECULARENABLE (see is_specular_emphasis): the
                    // BMT specular color and Phong power fill a
                    // D3DMATERIAL9 and the sun produces a classic
                    // fixed-function highlight. Only 19 hero-prop materials
                    // opt in; the power=0 outlier (one flower) is excluded
                    // since a zero exponent is degenerate in both models.
                    // Sheen variants wrap this base, so on EnvMap resources
                    // these values still govern the mask~0 texels.
                    material.specular_tint = self.specular;
                    material.reflectance = SPECULAR_EMPHASIS_REFLECTANCE;
                    material.perceptual_roughness = phong_power_to_perceptual_roughness(self.power);
                }
                if self.is_emissive() {
                    // self-illuminated panes/lamps (window lights, jewels)
                    // glow in their own texture colors; always on until a
                    // day/night cycle exists to drive it. The factor keeps
                    // the glow visible under daylight tonemapping.
                    material.emissive_texture = Some(material.base_color_texture.clone().unwrap());
                    material.emissive = LinearRgba::WHITE * EMISSIVE_BOOST;
                }
                if self.is_two_sided() {
                    // Bit1 (0x1) meshes are single-sided quads (talismans, hair,
                    // foliage, flags). double_sided flips the normal on back
                    // faces so both sides light correctly instead of showing a
                    // dark unlit back.
                    material.double_sided = true;
                    material.cull_mode = None;
                } else {
                    material.cull_mode = Some(Face::Back);
                }
                material
            }
            None => StandardMaterial::from(self.ambient),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_with(name: &str) -> JMXVBMT {
        JMXVBMT {
            materials: vec![SroMaterial {
                name: name.to_string(),
                ..default()
            }],
            diffuse_textures: HashMap::from([(material_label(name), Handle::default())]),
        }
    }

    /// One material record, in whichever of the two shapes the reader has to
    /// tell apart: `normal_map` appends the extra path plus its four bytes.
    fn material_bytes(name: &str, flag: u32, texture: &str, normal_map: Option<&str>) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&[0u8; 4 * 16]); // four RGBA colors
        bytes.extend_from_slice(&1.0f32.to_le_bytes()); // power
        bytes.extend_from_slice(&flag.to_le_bytes());
        bytes.extend_from_slice(&(texture.len() as u32).to_le_bytes());
        bytes.extend_from_slice(texture.as_bytes());
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&[32, 8, 0]);
        if let Some(path) = normal_map {
            bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
            bytes.extend_from_slice(path.as_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        bytes
    }

    /// A whole file body (count + records), as [`JMXVBMT::from`] takes it.
    fn set_bytes(records: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = (records.len() as u32).to_le_bytes().to_vec();
        for record in records {
            bytes.extend_from_slice(record);
        }
        bytes
    }

    /// One real material, then a record that is cut short. The reader must
    /// keep the first and stop — before the guard it panicked inside
    /// `get_fixed_size_string`, which is what one file of the user's own data
    /// (`avatar_w_ghostcaptain_2.bmt`) drives the plain reading into.
    #[test]
    fn a_truncated_record_stops_the_read_instead_of_panicking() {
        let mut bytes = set_bytes(&[material_bytes("wall", 0x141, "wall.ddj", None)]);
        bytes[..4].copy_from_slice(&2u32.to_le_bytes()); // two materials claimed
                                                         // the second record starts with a length prefix nothing can satisfy
        bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());

        let set = JMXVBMT::from(&bytes, PathBuf::from("prim/mtrl/test"));

        assert_eq!(set.materials.len(), 1, "the readable material is kept");
        assert_eq!(set.materials[0].name, "wall");
        assert_eq!(set.materials[0].diffuse_map_path, PathBuf::from("wall.ddj"));
        assert!(!set.materials[0].diffuse_map_is_absolute);
        assert_eq!(set.materials[0].normal_map_path, None);
    }

    /// The same flag word, the two shapes it is found in: 30 of the user's 34
    /// flagged files have no normal-map block, 3 have one. Whichever shape a
    /// file is in, every material has to come out, and the wrong reading fails
    /// in both directions — which is why the tail decides and not the material
    /// count. Read with the block, the first case stops after 1 of 2 the way
    /// `mob/arabia/karkadann.bmt` does; read without it, the second comes out
    /// *complete* with a 56-byte tail the way
    /// `mob/bossdungeon/mustafa_boss_dungeon_stone.BMT` does.
    #[test]
    fn a_flagged_file_reads_whole_in_either_shape() {
        const FLAG: u32 = 0x2345; // the word mustafa.bmt and karkadann.bmt share

        let without = set_bytes(&[
            material_bytes("mustafa", FLAG, "mustafa.ddj", None),
            material_bytes("mustafa_01", FLAG, "mustafa_01.ddj", None),
        ]);
        let set = JMXVBMT::from(&without, PathBuf::from("prim/mtrl/mob/arabia"));
        assert_eq!(set.materials.len(), 2);
        assert!(set.materials.iter().all(|m| m.has_bump_map()));
        assert!(set.materials.iter().all(|m| m.normal_map_path.is_none()));

        let with = set_bytes(&[
            material_bytes(
                "khulood",
                FLAG,
                "khulood.ddj",
                Some(r"prim\mtrl\khulood_n.ddj"),
            ),
            material_bytes(
                "khulood_01",
                FLAG,
                "khulood_01.ddj",
                Some("khulood_01_n.ddj"),
            ),
        ]);
        let set = JMXVBMT::from(&with, PathBuf::from("prim/mtrl/mob/bossdungeon"));
        assert_eq!(set.materials.len(), 2, "the second material is not lost");
        assert_eq!(
            set.materials[0].normal_map_path,
            Some(PathBuf::from(format!(
                "prim{sep}mtrl{sep}khulood_n.ddj",
                sep = std::path::MAIN_SEPARATOR
            ))),
            "the normal map path is read, separators and all"
        );
        assert_eq!(
            set.materials[1].normal_map_path,
            Some(PathBuf::from("khulood_01_n.ddj"))
        );
    }

    /// `prim/mtrl/mob/arabia/mustafa.bmt` is exact under neither shape: two
    /// bytes left without the block, cut short with it. The smaller tail wins,
    /// so both its materials survive and the tail stays unread.
    #[test]
    fn neither_shape_exact_keeps_the_reading_with_the_smaller_tail() {
        let mut bytes = set_bytes(&[
            material_bytes("mustafa", 0x2345, "mustafa.ddj", None),
            material_bytes("mustafa_01", 0x2345, "mustafa_01.ddj", None),
        ]);
        bytes.extend_from_slice(&[0, 0]); // the file tail nobody reads

        let set = JMXVBMT::from(&bytes, PathBuf::from("prim/mtrl/mob/arabia"));

        assert_eq!(set.materials.len(), 2);
        assert!(set.materials.iter().all(|m| m.normal_map_path.is_none()));
    }

    /// Meshes may spell a material differently from its set (`Gyo` vs `gyo`),
    /// and the on-demand variants must resolve it the way labels always have.
    #[test]
    fn material_lookup_is_case_insensitive() {
        let set = set_with("WaterGhost");
        assert!(set.material_with_texture("waterghost").is_some());
        assert!(set.material_with_texture("WATERGHOST").is_some());
        assert!(set.material_with_texture("other").is_none());
    }

    /// A material the loader skipped (no diffuse map) has no texture entry and
    /// so no variant — like the labeled sub-assets it never got.
    #[test]
    fn material_without_texture_has_no_variant() {
        let mut set = set_with("gyo");
        set.diffuse_textures.clear();
        assert!(set.material_with_texture("gyo").is_none());
    }

    /// A cut-out texture's alpha is its silhouette, so it may dither; an opaque
    /// material's alpha means nothing, so it must not start punching holes.
    /// The cutoff travels with the mode — proof the switch does not quietly
    /// re-author the original's alpha test (ref 0x80 = 0.5).
    #[test]
    fn only_a_cutout_follows_the_alpha_to_coverage_switch() {
        let cutout = SroMaterial {
            flag: 0x200,
            ..default()
        };
        let plain = SroMaterial::default();

        assert_eq!(
            cutout
                .to_masked_material(Handle::default(), false)
                .alpha_mode,
            AlphaMode::Mask(0.5),
            "off is the original's hard test"
        );
        assert_eq!(
            cutout
                .to_masked_material(Handle::default(), true)
                .alpha_mode,
            AlphaMode::AlphaToCoverage
        );
        for on in [false, true] {
            assert_eq!(
                plain.to_masked_material(Handle::default(), on).alpha_mode,
                AlphaMode::Opaque,
                "a material without an alpha channel stays opaque"
            );
        }
        assert_eq!(cutout_alpha_mode(0.25, false), AlphaMode::Mask(0.25));
    }

    /// The live switch moves a cut-out and nothing else: the sheen cut-out
    /// (`Mask(1/255)`, a sheen mask rather than transparency) and a plain
    /// blended material keep their modes, and flipping back restores the
    /// original's own threshold — the number `AlphaToCoverage` does not carry.
    #[test]
    fn the_live_retarget_moves_cutouts_and_leaves_every_other_threshold() {
        let mut materials = bevy::asset::Assets::<StandardMaterial>::default();
        let cutout = materials.add(StandardMaterial {
            alpha_mode: AlphaMode::Mask(CUTOUT_ALPHA_CUTOFF),
            ..default()
        });
        let sheen = materials.add(StandardMaterial {
            alpha_mode: AlphaMode::Mask(SHEEN_ALPHA_CUTOUT),
            ..default()
        });
        let blended = materials.add(StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..default()
        });

        assert_eq!(retarget_cutout_materials(&mut materials, true), 1);
        assert_eq!(
            materials.get(&cutout).unwrap().alpha_mode,
            AlphaMode::AlphaToCoverage
        );
        assert_eq!(
            materials.get(&sheen).unwrap().alpha_mode,
            AlphaMode::Mask(SHEEN_ALPHA_CUTOUT)
        );
        assert_eq!(
            materials.get(&blended).unwrap().alpha_mode,
            AlphaMode::Blend
        );

        // idempotent: a second run has nothing left to move
        assert_eq!(retarget_cutout_materials(&mut materials, true), 0);

        assert_eq!(retarget_cutout_materials(&mut materials, false), 1);
        assert_eq!(
            materials.get(&cutout).unwrap().alpha_mode,
            AlphaMode::Mask(CUTOUT_ALPHA_CUTOFF)
        );
    }

    /// The switch must not reach the sheen variants: there the alpha channel is
    /// a sheen mask (`SHEEN_ALPHA_CUTOUT` tests for exact zero), so coverage
    /// would read a dull shine as see-through metal.
    #[test]
    fn the_sheen_cutout_keeps_its_hard_test_while_the_switch_is_on() {
        let flag = CutoutAlphaToCoverage::default();
        flag.set(true);
        assert!(flag.enabled());

        let material = SroMaterial::default();
        let sheen = material.to_sheen_material(
            Handle::default(),
            SheenSettings::default(),
            true,
            (Handle::default(), Handle::default()),
        );
        assert_eq!(sheen.base.alpha_mode, AlphaMode::Mask(SHEEN_ALPHA_CUTOUT));
        assert_eq!(
            material
                .to_unsheened_material(Handle::default(), true)
                .alpha_mode,
            AlphaMode::Mask(SHEEN_ALPHA_CUTOUT)
        );
    }

    #[test]
    fn sheen_cutout_masks_exact_zero_alpha_and_plain_sheen_does_not() {
        let material = SroMaterial::default();
        let probes = || (Handle::default(), Handle::default());
        let settings = SheenSettings::default();

        let cutout = material.to_sheen_material(Handle::default(), settings, true, probes());
        assert_eq!(cutout.base.alpha_mode, AlphaMode::Mask(SHEEN_ALPHA_CUTOUT));
        assert_eq!(cutout.extension.settings.alpha_cutout, SHEEN_ALPHA_CUTOUT);

        let plain = material.to_sheen_material(Handle::default(), settings, false, probes());
        assert_eq!(plain.base.alpha_mode, AlphaMode::Opaque);
        assert_eq!(plain.extension.settings.alpha_cutout, settings.alpha_cutout);
    }
}
