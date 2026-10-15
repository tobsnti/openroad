//! worldmap_ncc — is our world-map sheet the original's art, and where on it
//! does a client screenshot sit?
//!
//! Idea: the claim this tool makes re-runnable is "the 363
//! `map_world_<x>x<z>.ddj` tiles stitch into the 4224x1408 sheet
//! `worldmap_mapinfo.txt` declares, and a client screenshot cross-correlates
//! against that sheet at NCC 0.92-0.96" (world-map RE report, 2026-10-09).
//! Nothing in the tree could produce that number: the one-off script is gone
//! and its atlas survives only at half resolution, so the number was
//! unsourced in our own terms. Here the sheet is rebuilt from the user's own
//! `Media.pk2` with our own readers — `bevy_pk2` for the archive,
//! `client::assets::ddj` for the pixels, `WorldMapTable` for the grid, so the
//! 363 and the 4224x1408 are *read from the data*, never written down — and
//! every number is printed next to the inputs that produced it.
//!
//! The measure is plain normalized cross-correlation of luminance, searched
//! over a 4-level box pyramid (8 -> 4 -> 2 -> 1) instead of an FFT: a full
//! sweep at 1/8 scale is cheap, its best peaks are refined level by level,
//! and no new crate is needed for it.
//!
//! Two controls always run, because the headline number alone says nothing:
//!   * **positive** — a crop of the sheet searched in the sheet must return
//!     r = 1.000 at its own offset. Anything else means the measure is
//!     broken, not the map.
//!   * **red** — the same measure between tiles that are *not* the same tile.
//!     If a wrong tile also scores 0.9, the headline number is worthless.
//!     Read that one first.

use std::path::{Path, PathBuf};

use bevy::asset::io::AssetReader;
use bevy_pk2::prelude::Archive;
use clap::{App, Arg};
use client::assets::ddj::{ddj_dds_payload, dds_to_rgba8};
use client::assets::textdata::decode::decode_textdata;
use client::assets::textdata::worldmap::{WorldMapTable, DU_PER_REGION};
use futures_lite::future::block_on;
use futures_lite::io::AsyncReadExt;
use image::imageops::FilterType;
use image::RgbImage;
use serde_json::{json, Value};

/// Entry points inside Media.pk2 (the client loads the same two through
/// `media://`, see `client/src/plugins/textdata.rs`).
const MAPINFO: &str = "server_dep/silkroad/textdata/worldmap_mapinfo.txt";
const LOCALINFO: &str = "server_dep/silkroad/textdata/worldmap_localinfo.txt";

/// Luminance plane. `f32` because the correlation sums are the hot loop and
/// the inputs are 8-bit anyway.
struct Gray {
    w: usize,
    h: usize,
    px: Vec<f32>,
}

impl Gray {
    /// Rec.601 luma over tightly packed RGB8, the same weights the measurement
    /// this tool re-runs used.
    fn from_rgb(w: usize, h: usize, rgb: &[u8]) -> Self {
        let px = rgb
            .chunks_exact(3)
            .map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32)
            .collect();
        Gray { w, h, px }
    }

    fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Gray {
        let mut px = vec![0.0f32; w * h];
        for row in 0..h {
            let src = (y + row) * self.w + x;
            px[row * w..row * w + w].copy_from_slice(&self.px[src..src + w]);
        }
        Gray { w, h, px }
    }

    /// Box-average downsample by `f`. Box, not bilinear: it is the average of
    /// exactly the texels that fall in the cell, so the pyramid cannot invent
    /// structure that would move a peak.
    fn boxed(&self, f: usize) -> Gray {
        let (w, h) = (self.w / f, self.h / f);
        let mut px = vec![0.0f32; w * h];
        let n = (f * f) as f32;
        for y in 0..h {
            for x in 0..w {
                let mut s = 0.0;
                for dy in 0..f {
                    let row = (y * f + dy) * self.w + x * f;
                    for dx in 0..f {
                        s += self.px[row + dx];
                    }
                }
                px[y * w + x] = s / n;
            }
        }
        Gray { w, h, px }
    }

    fn stats(&self) -> (f64, f64) {
        let n = self.px.len() as f64;
        let mean = self.px.iter().map(|v| *v as f64).sum::<f64>() / n;
        let var = self
            .px
            .iter()
            .map(|v| (*v as f64 - mean).powi(2))
            .sum::<f64>()
            / n;
        (mean, var.sqrt())
    }
}

/// A template with its mean removed once, so the inner loop only needs the
/// window's own sums.
struct Tpl {
    w: usize,
    h: usize,
    centered: Vec<f32>,
    norm: f64,
}

impl Tpl {
    /// A template with no luminance structure correlates with nothing: its
    /// norm is ~0, every position divides by ~0 and the whole sweep returns
    /// 0.0000 — a run that looks like a measured "no match" but measured
    /// nothing. Measured 2026-10-15 with a template that fell entirely on a
    /// gallery shot's flat background. Callers that take their rect from a
    /// human must check this.
    fn is_flat(g: &Gray) -> bool {
        g.stats().1 < 1.0
    }

    fn new(g: &Gray) -> Tpl {
        let n = g.px.len() as f64;
        let mean = (g.px.iter().map(|v| *v as f64).sum::<f64>() / n) as f32;
        let centered: Vec<f32> = g.px.iter().map(|v| v - mean).collect();
        let norm = centered
            .iter()
            .map(|v| (*v as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        Tpl {
            w: g.w,
            h: g.h,
            centered,
            norm,
        }
    }
}

/// NCC of `tpl` against the window of `img` whose top-left is (x, y).
/// Flat windows (zero variance) score 0 rather than dividing by ~0.
fn ncc_at(img: &Gray, tpl: &Tpl, x: usize, y: usize) -> f64 {
    let (mut s1, mut s2, mut cross) = (0.0f64, 0.0f64, 0.0f64);
    for row in 0..tpl.h {
        let i = (y + row) * img.w + x;
        let t = row * tpl.w;
        for col in 0..tpl.w {
            let v = img.px[i + col] as f64;
            s1 += v;
            s2 += v * v;
            cross += v * tpl.centered[t + col] as f64;
        }
    }
    let n = (tpl.w * tpl.h) as f64;
    let den = (s2 - s1 * s1 / n).max(1e-9).sqrt() * tpl.norm;
    if den <= 1e-9 {
        return 0.0;
    }
    cross / den
}

/// A disk (in full-resolution sheet pixels) that a search must stay out of.
/// This is what turns "the peak is sharp" from a claim into a measurement: the
/// same template is searched again with its own winner fenced off, so the
/// number that comes back is the best the sheet can do *somewhere else*.
#[derive(Clone, Copy)]
struct Exclude {
    x: usize,
    y: usize,
    r: usize,
}

impl Exclude {
    /// `(x, y)` is a position at pyramid level factor `f`.
    fn hits(&self, x: usize, y: usize, f: usize) -> bool {
        (x * f).abs_diff(self.x) < self.r && (y * f).abs_diff(self.y) < self.r
    }
}

/// Exhaustive NCC map at one level, returned as (r, x, y) peaks after
/// non-maximum suppression.
fn sweep(
    img: &Gray,
    tpl: &Tpl,
    keep: usize,
    nms: usize,
    f: usize,
    skip: Option<Exclude>,
) -> Vec<(f64, usize, usize)> {
    if img.w < tpl.w || img.h < tpl.h {
        return Vec::new();
    }
    let mut all = Vec::with_capacity((img.w - tpl.w + 1) * (img.h - tpl.h + 1));
    for y in 0..=img.h - tpl.h {
        for x in 0..=img.w - tpl.w {
            if skip.is_some_and(|s| s.hits(x, y, f)) {
                continue;
            }
            all.push((ncc_at(img, tpl, x, y), x, y));
        }
    }
    all.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out: Vec<(f64, usize, usize)> = Vec::new();
    for cand in all {
        if out.len() >= keep {
            break;
        }
        if out
            .iter()
            .all(|(_, x, y)| cand.1.abs_diff(*x) >= nms || cand.2.abs_diff(*y) >= nms)
        {
            out.push(cand);
        }
    }
    out
}

/// Candidates kept per pyramid level, index = level index (finest first):
/// a wide net where it is cheap, few windows where each one costs 40 000
/// multiply-adds.
const KEEP: [usize; 4] = [8, 16, 32, 64];

/// How far "somewhere else" starts for the off-peak control, in sheet pixels.
/// 64 is half a map tile: far enough that the winner's own slope cannot be
/// mistaken for a rival, close enough that a neighbouring tile still counts.
const OFF_PEAK_RADIUS: usize = 64;

/// Search `tpl` in `sheet` through the box pyramid and return
/// (r, x, y) at full resolution, best first.
fn pyramid_search(
    sheet: &[Gray],
    tpl_levels: &[Tpl],
    levels: &[usize],
    keep: &[usize],
    skip: Option<Exclude>,
) -> Vec<(f64, usize, usize)> {
    // coarsest level: one full sweep, widest candidate set
    let top = levels.len() - 1;
    let mut cands = sweep(
        &sheet[top],
        &tpl_levels[top],
        keep[top],
        2,
        levels[top],
        skip,
    );
    for li in (0..levels.len() - 1).rev() {
        let step = levels[li + 1] / levels[li];
        let img = &sheet[li];
        let tpl = &tpl_levels[li];
        let mut next: Vec<(f64, usize, usize)> = Vec::new();
        for (_, cx, cy) in &cands {
            // the coarse peak locates the window to +-1 coarse pixel
            let x0 = (cx * step).saturating_sub(step);
            let y0 = (cy * step).saturating_sub(step);
            for y in y0..=(y0 + 2 * step) {
                for x in x0..=(x0 + 2 * step) {
                    if x + tpl.w <= img.w
                        && y + tpl.h <= img.h
                        && !skip.is_some_and(|s| s.hits(x, y, levels[li]))
                    {
                        next.push((ncc_at(img, tpl, x, y), x, y));
                    }
                }
            }
        }
        next.sort_by(|a, b| b.0.total_cmp(&a.0));
        let budget = keep[li];
        let mut pruned: Vec<(f64, usize, usize)> = Vec::new();
        for cand in next {
            if pruned.len() >= budget {
                break;
            }
            if pruned
                .iter()
                .all(|(_, x, y)| cand.1.abs_diff(*x) >= 2 || cand.2.abs_diff(*y) >= 2)
            {
                pruned.push(cand);
            }
        }
        cands = pruned;
    }
    cands
}

fn read_entry(archive: &Archive, path: &str) -> Result<Vec<u8>, String> {
    let mut reader = block_on(archive.read(Path::new(path))).map_err(|e| format!("{path}: {e}"))?;
    let mut data = Vec::new();
    block_on(async { reader.read_to_end(&mut data).await }).map_err(|e| format!("{path}: {e}"))?;
    Ok(data)
}

fn parse_rect(s: &str) -> Result<(usize, usize, usize, usize), String> {
    let n: Vec<usize> = s
        .split(',')
        .map(|p| p.trim().parse::<usize>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    match n.as_slice() {
        [x, y, w, h] => Ok((*x, *y, *w, *h)),
        _ => Err(format!("expected x,y,w,h — got '{s}'")),
    }
}

fn main() -> Result<(), String> {
    let m = App::new("worldmap_ncc")
        .version("0.1.0")
        .about("Stitch the world-map sheet from Media.pk2 and cross-correlate a screenshot against it")
        .arg(Arg::with_name("pk2").long("pk2").takes_value(true).required(true)
            .help("Path to Media.pk2 (the key comes from config.yaml / SRO_PK2_KEY, never from here)"))
        .arg(Arg::with_name("map-id").long("map-id").takes_value(true).default_value("0")
            .help("WLocalmap id to stitch (0 = world map)"))
        .arg(Arg::with_name("shot").long("shot").takes_value(true)
            .help("Client screenshot (PNG) to locate on the sheet"))
        .arg(Arg::with_name("shot-scale").long("shot-scale").takes_value(true).default_value("1.0")
            .help("HUD scale the screenshot was taken at; templates are divided by it"))
        .arg(Arg::with_name("template").long("template").takes_value(true)
            .multiple(true).number_of_values(1)
            .help("x,y,w,h in screenshot pixels; repeatable, one comparison pair each"))
        .arg(Arg::with_name("atlas-out").long("atlas-out").takes_value(true)
            .help("Write the stitched sheet as PNG (not for committing — it is PK2 art)"))
        .arg(Arg::with_name("compare-sheet").long("compare-sheet").takes_value(true)
            .help("Reference sheet image to check the stitch against (any scale)"))
        .arg(Arg::with_name("red-tiles").long("red-tiles").takes_value(true).default_value("12")
            .help("Tiles sampled for the wrong-tile red control"))
        .arg(Arg::with_name("json").long("json").takes_value(true)
            .help("Write every measured number as JSON"))
        .get_matches();

    let pk2 = PathBuf::from(m.value_of("pk2").unwrap());
    if !pk2.is_file() {
        return Err(format!(
            "no archive at {} — this tool measures the user's own PK2 or nothing",
            pk2.display()
        ));
    }
    let map_id: u32 = m
        .value_of("map-id")
        .unwrap()
        .parse()
        .map_err(|_| "bad --map-id")?;
    let scale: f32 = m
        .value_of("shot-scale")
        .unwrap()
        .parse()
        .map_err(|_| "bad --shot-scale")?;
    let mut report = json!({});

    // --- readiness: the archive, and how much of it we can see --------------
    let archive = Archive::configured(&pk2);
    let all = archive.root.get_all_entries();
    let files = all.iter().filter(|(_, e)| e.is_file()).count();
    println!("archive  {}: {files} file entries", pk2.display());
    report["archive"] = json!({ "path": pk2.display().to_string(), "file_entries": files });

    // --- the grid comes from the data, not from this file -------------------
    let mapinfo = decode_textdata(&read_entry(&archive, MAPINFO)?);
    let localinfo = decode_textdata(&read_entry(&archive, LOCALINFO)?);
    let table = WorldMapTable::parse(&mapinfo, &localinfo);
    let map = table
        .map(map_id)
        .ok_or_else(|| format!("no WLocalmap row with id {map_id}"))?;
    if !map.tiled {
        return Err(format!("map {map_id} is a single image, not a tile grid"));
    }
    let k = map.px_per_du();
    let tile_px = (4.0 * DU_PER_REGION * k).round() as usize;
    let (sheet_w, sheet_h) = (
        map.logical_w.round() as usize,
        map.logical_h.round() as usize,
    );
    println!(
        "mapinfo  {MAPINFO}: map {map_id} prefix '{}' sheet {sheet_w}x{sheet_h} \
         regions x {}..{} z {}..{} tile {tile_px}px",
        map.texture, map.x1, map.x2, map.z2, map.z1
    );

    // --- stitch -------------------------------------------------------------
    let mut sheet_rgb = vec![0u8; sheet_w * sheet_h * 3];
    let mut names: Vec<(String, usize, usize)> = Vec::new();
    let mut tz = map.z2 + 3;
    while tz <= map.z1 {
        let mut tx = map.x1;
        while tx + 3 <= map.x2 {
            let o = map.project(tx as f32 * DU_PER_REGION, (tz + 1) as f32 * DU_PER_REGION);
            names.push((
                format!("{}{}x{}.ddj", map.texture, tx, tz),
                o.x.round() as usize,
                o.y.round() as usize,
            ));
            tx += 4;
        }
        tz += 4;
    }
    let expected = names.len();
    let mut decoded = 0usize;
    let mut missing: Vec<String> = Vec::new();
    let mut blank: Vec<String> = Vec::new();
    for (name, ox, oy) in &names {
        let Ok(bytes) = read_entry(&archive, name) else {
            missing.push(name.clone());
            continue;
        };
        let payload =
            ddj_dds_payload(&bytes).ok_or_else(|| format!("{name}: not a DDJ container"))?;
        let (tw, th, rgba) =
            dds_to_rgba8(payload).ok_or_else(|| format!("{name}: no CPU decode for its format"))?;
        if tw as usize != tile_px || th as usize != tile_px {
            return Err(format!(
                "{name}: {tw}x{th}, but the sheet grid wants {tile_px}"
            ));
        }
        let mut lit = false;
        for y in 0..th as usize {
            for x in 0..tw as usize {
                let s = (y * tw as usize + x) * 4;
                let d = ((oy + y) * sheet_w + ox + x) * 3;
                sheet_rgb[d] = rgba[s];
                sheet_rgb[d + 1] = rgba[s + 1];
                sheet_rgb[d + 2] = rgba[s + 2];
                lit |= (rgba[s] | rgba[s + 1] | rgba[s + 2]) != 0;
            }
        }
        if !lit {
            blank.push(name.clone());
        }
        decoded += 1;
    }
    let sheet = Gray::from_rgb(sheet_w, sheet_h, &sheet_rgb);
    let (mean, sd) = sheet.stats();
    println!(
        "stitch   {decoded}/{expected} tiles decoded, {} missing, {} all-black; \
         sheet luma mean {mean:.1} sd {sd:.1}",
        missing.len(),
        blank.len()
    );
    report["stitch"] = json!({
        "tiles_expected": expected, "tiles_decoded": decoded,
        "missing": missing, "all_black": blank,
        "sheet": [sheet_w, sheet_h], "luma_mean": mean, "luma_sd": sd,
        "first_tile": names.first().map(|n| n.0.clone()),
        "last_tile": names.last().map(|n| n.0.clone()),
    });
    if decoded == 0 {
        return Err("no tile decoded — nothing was measured".into());
    }

    if let Some(out) = m.value_of("atlas-out") {
        let img = RgbImage::from_raw(sheet_w as u32, sheet_h as u32, sheet_rgb.clone())
            .ok_or("sheet buffer does not match its size")?;
        img.save(out).map_err(|e| e.to_string())?;
        println!("atlas    {out}");
    }

    // --- pyramid of the sheet ----------------------------------------------
    let levels = [1usize, 2, 4, 8];
    let sheet_levels: Vec<Gray> = levels
        .iter()
        .map(|f| {
            if *f == 1 {
                sheet.crop(0, 0, sheet_w, sheet_h)
            } else {
                sheet.boxed(*f)
            }
        })
        .collect();

    // --- control 1 (positive): the sheet against itself ---------------------
    let (px, py) = (sheet_w / 2, sheet_h / 3);
    let probe = sheet.crop(px, py, 200, 200);
    let probe_levels: Vec<Tpl> = levels
        .iter()
        .map(|f| {
            Tpl::new(&if *f == 1 {
                probe.crop(0, 0, 200, 200)
            } else {
                probe.boxed(*f)
            })
        })
        .collect();
    let self_hit = pyramid_search(&sheet_levels, &probe_levels, &levels, &KEEP, None);
    let (sr, sx, sy) = self_hit.first().copied().unwrap_or((f64::NAN, 0, 0));
    println!(
        "CONTROL+ sheet crop 200x200 at ({px},{py}) found at ({sx},{sy}) r={sr:.4}  \
         [must be ({px},{py}) r=1.0000]"
    );
    report["control_positive"] = json!({"crop": [px, py, 200, 200], "found": [sx, sy], "r": sr});

    // --- control 2 (red): a WRONG tile ------------------------------------
    let n_red: usize = m
        .value_of("red-tiles")
        .unwrap()
        .parse()
        .map_err(|_| "bad --red-tiles")?;
    let step = (names.len() / n_red.max(1)).max(1);
    let picks: Vec<&(String, usize, usize)> = names.iter().step_by(step).take(n_red).collect();
    let mut own: Vec<f64> = Vec::new();
    let mut wrong: Vec<(f64, String, String)> = Vec::new();
    for (name, ox, oy) in &picks {
        let tpl = Tpl::new(&sheet.crop(*ox, *oy, tile_px, tile_px));
        own.push(ncc_at(&sheet, &tpl, *ox, *oy));
        for (other, qx, qy) in &picks {
            if other == name {
                continue;
            }
            wrong.push((ncc_at(&sheet, &tpl, *qx, *qy), name.clone(), other.clone()));
        }
    }
    wrong.sort_by(|a, b| b.0.total_cmp(&a.0));
    let own_min = own.iter().copied().fold(f64::INFINITY, f64::min);
    let w_max = wrong.first().map(|w| w.0).unwrap_or(f64::NAN);
    let w_mean = wrong.iter().map(|w| w.0).sum::<f64>() / wrong.len() as f64;
    println!(
        "CONTROL- {} tiles, {} wrong-tile pairs: r max {:.4} ({} at {}), mean {:.4}; \
         same-tile r min {:.4}",
        picks.len(),
        wrong.len(),
        w_max,
        wrong[0].1.rsplit('/').next().unwrap_or(""),
        wrong[0].2.rsplit('/').next().unwrap_or(""),
        w_mean,
        own_min
    );
    report["control_red_tiles"] = json!({
        "tiles": picks.len(), "pairs": wrong.len(),
        "wrong_max": w_max, "wrong_mean": w_mean, "same_min": own_min,
        "wrong_max_pair": [wrong[0].1.clone(), wrong[0].2.clone()],
    });

    // --- control 3: the stitch against a reference sheet image -------------
    if let Some(refp) = m.value_of("compare-sheet") {
        let img = image::open(refp).map_err(|e| e.to_string())?.to_rgb8();
        let (rw, rh) = (img.width() as usize, img.height() as usize);
        let g = Gray::from_rgb(rw, rh, img.as_raw());
        let f = sheet_w / rw.max(1);
        let ours = if f <= 1 {
            sheet.crop(0, 0, sheet_w, sheet_h)
        } else {
            sheet.boxed(f)
        };
        let r = if ours.w == g.w && ours.h == g.h {
            let tpl = Tpl::new(&g);
            ncc_at(&ours, &tpl, 0, 0)
        } else {
            f64::NAN
        };
        println!("CONTROL= reference {refp} {rw}x{rh} vs our sheet /{f}: r={r:.4}");
        report["control_reference_sheet"] =
            json!({"path": refp, "size": [rw, rh], "our_divisor": f, "r": r});
    }

    // --- the measurement ---------------------------------------------------
    if let Some(shot) = m.value_of("shot") {
        let img = image::open(shot).map_err(|e| e.to_string())?.to_rgb8();
        println!(
            "shot     {shot} {}x{} at scale {scale} -> sheet pixels /{scale}",
            img.width(),
            img.height()
        );
        let rects: Vec<(usize, usize, usize, usize)> = m
            .values_of("template")
            .map(|v| v.map(parse_rect).collect::<Result<Vec<_>, _>>())
            .transpose()?
            .unwrap_or_default();
        if rects.is_empty() {
            return Err("--shot needs at least one --template x,y,w,h".into());
        }
        let mut rows = Vec::new();
        for (x, y, w, h) in rects {
            let cut =
                image::imageops::crop_imm(&img, x as u32, y as u32, w as u32, h as u32).to_image();
            let (tw, th) = (
                (w as f32 / scale).round() as u32,
                (h as f32 / scale).round() as u32,
            );
            let small = image::imageops::resize(&cut, tw, th, FilterType::Triangle);
            let tg = Gray::from_rgb(tw as usize, th as usize, small.as_raw());
            if Tpl::is_flat(&tg) {
                return Err(format!(
                    "template {x},{y},{w},{h} is a flat patch (luma sd {:.2}) — \
                     it carries no structure to correlate, so this would measure nothing",
                    tg.stats().1
                ));
            }
            let tpl_levels: Vec<Tpl> = levels
                .iter()
                .map(|f| {
                    Tpl::new(&if *f == 1 {
                        tg.crop(0, 0, tg.w, tg.h)
                    } else {
                        tg.boxed(*f)
                    })
                })
                .collect();
            let hits = pyramid_search(&sheet_levels, &tpl_levels, &levels, &KEEP, None);
            let (r, mx, my) = hits.first().copied().unwrap_or((f64::NAN, 0, 0));
            // where the screenshot's own (0,0) would land on the sheet
            let ox = mx as f64 - x as f64 / scale as f64;
            let oy = my as f64 - y as f64 / scale as f64;
            // red control for this pair: search the same template again with
            // its own winner fenced off, so "the peak is sharp" is measured
            // rather than asserted.
            let far = pyramid_search(
                &sheet_levels,
                &tpl_levels,
                &levels,
                &KEEP,
                Some(Exclude {
                    x: mx,
                    y: my,
                    r: OFF_PEAK_RADIUS,
                }),
            )
            .first()
            .copied();
            println!(
                "TEMPLATE {x},{y},{w},{h} -> {tw}x{th} sheet px: r={r:.4} at ({mx},{my}), \
                 implied shot origin ({ox:.2},{oy:.2}){}",
                match far {
                    Some((fr, fx, fy)) =>
                        format!("; best r outside {OFF_PEAK_RADIUS}px: {fr:.4} at ({fx},{fy})"),
                    None => ", sheet too small for an off-peak search".to_string(),
                }
            );
            rows.push(json!({
                "template_shot_px": [x, y, w, h],
                "template_sheet_px": [tw, th],
                "r": r, "at": [mx, my], "implied_origin": [ox, oy],
                "off_peak": far.map(|(fr, fx, fy)| json!({"r": fr, "at": [fx, fy]})),
            }));
        }
        let rs: Vec<f64> = rows
            .iter()
            .map(|r| r["r"].as_f64().unwrap_or(f64::NAN))
            .collect();
        let xs: Vec<f64> = rows
            .iter()
            .map(|r| r["implied_origin"][0].as_f64().unwrap_or(f64::NAN))
            .collect();
        let ys: Vec<f64> = rows
            .iter()
            .map(|r| r["implied_origin"][1].as_f64().unwrap_or(f64::NAN))
            .collect();
        let spread = |v: &[f64]| {
            v.iter().copied().fold(f64::MIN, f64::max) - v.iter().copied().fold(f64::MAX, f64::min)
        };
        println!(
            "SUMMARY  {} pairs: r {:.4}..{:.4}; implied origin spread x {:.2} px, y {:.2} px",
            rows.len(),
            rs.iter().copied().fold(f64::MAX, f64::min),
            rs.iter().copied().fold(f64::MIN, f64::max),
            spread(&xs),
            spread(&ys)
        );
        report["templates"] = Value::Array(rows);
        report["summary"] = json!({
            "pairs": rs.len(),
            "r_min": rs.iter().copied().fold(f64::MAX, f64::min),
            "r_max": rs.iter().copied().fold(f64::MIN, f64::max),
            "origin_spread_px": [spread(&xs), spread(&ys)],
        });
    }

    if let Some(p) = m.value_of("json") {
        std::fs::write(p, serde_json::to_string_pretty(&report).unwrap())
            .map_err(|e| e.to_string())?;
        println!("json     {p}");
    }
    Ok(())
}
