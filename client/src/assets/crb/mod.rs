//! `.crb` — the 16x16 crest a guild or alliance draws for itself.
//!
//! Idea: a crest is not an image file. It is 256 raw palette indices, one byte
//! per pixel, with no header, no size and no palette in it — the length *is*
//! the format. Two things therefore have to be supplied from outside the file,
//! and both are the point of this module:
//!
//! * **Rows run bottom to top.** Byte 0 is the bottom-left pixel, byte 255 the
//!   top-right one, the way an uncompressed Windows DIB is stored.
//! * **The colours are the Windows system 256-colour palette** — ten static
//!   system colours, 236 halftone entries, ten more static colours. It is an OS
//!   table, so the game never ships it and we carry a copy next to this file.
//!
//! Both come from JellyBitz/Crest2Bitmap (MIT, portable), and both hold up
//! against the 31 crests in the user's own vSRO server files: decoded bottom-up
//! with this palette they are coherent artwork (a Half-Life lambda, a cat, two
//! smileys), decoded top-down they are upside down.
//!
//! Where the client needs it: the original formats the file name itself, as
//! `"G%u_%u_%u.crb"` (`sro_client.exe` VA 0xde176c, used at 0x9a48b0) for a
//! guild crest and `"A%u_%u_%u.crb"` (VA 0xde177c, used at 0x9a4887) for an
//! alliance crest — a session global we have not identified, then the id and
//! the crest revision, which are the wire fields we already read in the
//! entity-spawn tail (`client/src/net/reader.rs:163`: crest rev, union id,
//! union crest rev) and in `GuildPlayerLog`. The same function allocates
//! 0x100 bytes for the pixel buffer (0x7983ea), which is where the 256 below
//! comes from independently of the MIT tool. The consumers that still draw
//! nothing are the guild window, the name tag next to a guild name, the arena
//! scoreboard's crest cell (`plugins/hud/arena/scoreboard.rs`) and the world
//! map's siege-fortress crest (`assets/textdata/siegefortressreward.rs`).

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use thiserror::Error;

/// Edge length in pixels.
pub const CREST_SIDE: usize = 16;
/// Length of a crest file, one palette index per pixel. Nothing else
/// identifies the format, so this doubles as its signature.
pub const CREST_LEN: usize = CREST_SIDE * CREST_SIDE;

/// 256 RGB triples, byte-identical to `Resources/Palette_System_Windows.ACT`
/// in JellyBitz/Crest2Bitmap (MIT),
/// sha256 a15e9392b2f59d20b29227282ab7c50ccd4623d5492a832b888ee23003de75cc.
const PALETTE: &[u8; 3 * 256] = include_bytes!("palette_system_windows.act");

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CrestError {
    #[error("a crest file is {} bytes, got {0}", CREST_LEN)]
    WrongLength(usize),
}

/// A decoded crest in RGBA8, **top row first** — the file's own order is
/// reversed here once so that every consumer reads it like any other image.
#[derive(Clone)]
pub struct Crest {
    pixels: [u8; CREST_LEN * 4],
}

/// Turns the 256 indices of a `.crb` into RGBA8, flipping the row order.
pub fn decode(bytes: &[u8]) -> Result<Crest, CrestError> {
    if bytes.len() != CREST_LEN {
        return Err(CrestError::WrongLength(bytes.len()));
    }

    let mut pixels = [0u8; CREST_LEN * 4];
    for (offset, &index) in bytes.iter().enumerate() {
        let x = offset % CREST_SIDE;
        let y = CREST_SIDE - 1 - offset / CREST_SIDE;
        let source = usize::from(index) * 3;
        let target = (y * CREST_SIDE + x) * 4;
        pixels[target..target + 3].copy_from_slice(&PALETTE[source..source + 3]);
        pixels[target + 3] = u8::MAX;
    }
    Ok(Crest { pixels })
}

impl Crest {
    /// RGBA at `x` from the left and `y` from the **top**.
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let at = (y * CREST_SIDE + x) * 4;
        self.pixels[at..at + 4]
            .try_into()
            .expect("4 bytes per pixel")
    }

    #[allow(dead_code)]
    pub fn to_image(&self) -> Image {
        Image::new(
            Extent3d {
                width: CREST_SIDE as u32,
                height: CREST_SIDE as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            self.pixels.to_vec(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{decode, CrestError, CREST_LEN, CREST_SIDE, PALETTE};

    fn palette_color(index: u8) -> [u8; 3] {
        let at = usize::from(index) * 3;
        PALETTE[at..at + 3].try_into().expect("3 bytes per entry")
    }

    /// The directory of real `.crb` files, if the runner has one. They are
    /// player-drawn content out of a server's `files/Crest/`, so none of them
    /// may live in this repository.
    fn corpus() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("OPENROAD_CREST_DIR")?);
        dir.is_dir().then_some(dir)
    }

    /// Guards the palette file against being swapped or truncated: these seven
    /// entries are the Windows static system colours that bracket the halftone
    /// block, at the indices the system palette puts them.
    #[test]
    fn the_palette_is_the_windows_system_palette() {
        assert_eq!(palette_color(0), [0, 0, 0]);
        assert_eq!(palette_color(7), [128, 128, 128]);
        assert_eq!(palette_color(8), [192, 220, 192]);
        assert_eq!(palette_color(9), [166, 202, 240]);
        assert_eq!(palette_color(246), [255, 251, 240]);
        assert_eq!(palette_color(249), [255, 0, 0]);
        assert_eq!(palette_color(255), [255, 255, 255]);
    }

    #[test]
    fn only_a_full_crest_decodes() {
        assert_eq!(decode(&[]).err(), Some(CrestError::WrongLength(0)));
        assert_eq!(
            decode(&[0; CREST_LEN - 1]).err(),
            Some(CrestError::WrongLength(CREST_LEN - 1))
        );
        assert!(decode(&[0; CREST_LEN]).is_ok());
    }

    /// Pins the row flip at one pixel: byte 0 is the bottom-left pixel, so it
    /// has to come out at the *last* row of the decoded image.
    #[test]
    fn the_first_byte_is_the_bottom_left_pixel() {
        let mut bytes = [255u8; CREST_LEN];
        bytes[0] = 249;

        let crest = decode(&bytes).expect("full crest");
        assert_eq!(crest.pixel(0, CREST_SIDE - 1), [255, 0, 0, 255]);
        assert_eq!(crest.pixel(0, 0), [255, 255, 255, 255]);
    }

    /// Needs `OPENROAD_CREST_DIR` pointing at a directory of real `.crb`
    /// files; skips silently otherwise, so a checkout without game data stays
    /// green. Measured 2026-10-28 over a vSRO server's `files/Crest/`: 31
    /// files, all exactly 256 bytes.
    #[test]
    fn every_crest_in_the_corpus_decodes() {
        let Some(dir) = corpus() else {
            eprintln!("OPENROAD_CREST_DIR not set; skipping the .crb corpus sweep");
            return;
        };

        let mut decoded = 0_usize;
        for entry in std::fs::read_dir(&dir).expect("readable crest directory") {
            let path = entry.expect("readable entry").path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("crb"))
            {
                continue;
            }
            let bytes = std::fs::read(&path).expect("readable crest");
            decode(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            decoded += 1;
        }
        assert!(decoded > 0, "no .crb files in {}", dir.display());
    }

    /// The orientation claim against real artwork rather than a constructed
    /// buffer. `G99_463_1.crb` is a smiley blowing a red heart, and the heart
    /// is the only strongly red thing in it: 24 pixels, all of them in the
    /// lower half once the rows are read bottom-up. Read top-down the same 24
    /// pixels land in the upper half, which is the control.
    #[test]
    fn a_real_crest_stands_upright_only_bottom_up() {
        let Some(path) = corpus().map(|dir| dir.join("G99_463_1.crb")) else {
            eprintln!("OPENROAD_CREST_DIR not set; skipping the upright check");
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("{} not present; skipping the upright check", path.display());
            return;
        };

        let crest = decode(&bytes).expect("full crest");
        let mut red_rows = Vec::new();
        for y in 0..CREST_SIDE {
            for x in 0..CREST_SIDE {
                let [r, g, b, _] = crest.pixel(x, y);
                if i16::from(r) - i16::from(g.max(b)) > 100 {
                    red_rows.push(y);
                }
            }
        }

        assert_eq!(red_rows.len(), 24, "the heart is 24 pixels");
        assert!(
            red_rows.iter().all(|&y| y >= CREST_SIDE / 2),
            "the heart must sit in the lower half, rows were {red_rows:?}"
        );
    }
}
