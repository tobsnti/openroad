use bevy::color::Srgba;
use std::collections::HashMap;
use std::path::{PathBuf, MAIN_SEPARATOR_STR};
use std::str::FromStr;

use bevy::math::Rect;
use bevy::prelude::{warn, Asset, Color, JustifyContent, Vec2};
use bevy::reflect::TypePath;
use bevy::ui::AlignSelf;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod loader;

#[derive(Serialize, Deserialize, Asset, TypePath)]
pub struct InterfaceText(pub Vec<Section>);

#[derive(Serialize, Deserialize)]
pub struct Section {
    pub(crate) name: String,
    pub(crate) structs: Vec<Element>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Element {
    pub(crate) name: String,
    pub(crate) typ: String,
    pub(crate) entries: HashMap<String, Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum Value {
    Integer(i32),
    Str(String),
    Point {
        x: f32,
        y: f32,
    },
    Rect {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    Color {
        a: i32,
        r: i32,
        g: i32,
        b: i32,
    },
}

/// Why a resinfo line could not be read. Every one of these used to be a
/// `panic!` in the load path, which is a client crash on one malformed line
/// in a 247-file corpus that provably contains malformed lines.
#[derive(Error, Debug, PartialEq)]
pub enum ResinfoError {
    #[error("line {line}: unknown value type {typ:?}")]
    UnknownType { line: usize, typ: String },
    #[error("line {line}: {typ} value {value:?} is malformed")]
    MalformedValue {
        line: usize,
        typ: String,
        value: String,
    },
}

/// Unescape the two-character `\n` sequences the corpus stores inside string
/// values (1160 occurrences across 28 files). Read literally, every
/// multi-line string collapses into one run-on line.
fn unescape(value: &str) -> String {
    value.replace("\\n", "\n")
}

impl Value {
    /// Parse one `Key=TYPE,"value"` payload. Fallible by design — see
    /// [`ResinfoError`].
    pub fn parse(typ: &str, value: &str, line: usize) -> Result<Self, ResinfoError> {
        let numbers = |count: usize| -> Option<Vec<f32>> {
            let parsed: Vec<f32> = value
                .split(',')
                .map(|n| f32::from_str(n.trim()).ok())
                .collect::<Option<Vec<f32>>>()?;
            (parsed.len() == count).then_some(parsed)
        };
        let malformed = || ResinfoError::MalformedValue {
            line,
            typ: typ.to_string(),
            value: value.to_string(),
        };
        match typ {
            "INTEGER" => Ok(Value::Integer(
                value.trim().parse::<i32>().map_err(|_| malformed())?,
            )),
            "STRING" => Ok(Value::Str(unescape(value))),
            "POINT" => {
                let n = numbers(2).ok_or_else(malformed)?;
                Ok(Value::Point { x: n[0], y: n[1] })
            }
            "RECT" => {
                let n = numbers(4).ok_or_else(malformed)?;
                Ok(Value::Rect {
                    x: n[0] as i32,
                    y: n[1] as i32,
                    width: n[2] as i32,
                    height: n[3] as i32,
                })
            }
            "COLOR" => {
                let n = numbers(4).ok_or_else(malformed)?;
                Ok(Value::Color {
                    a: n[0] as i32,
                    r: n[1] as i32,
                    g: n[2] as i32,
                    b: n[3] as i32,
                })
            }
            _ => Err(ResinfoError::UnknownType {
                line,
                typ: typ.to_string(),
            }),
        }
    }
}

#[allow(dead_code)]
impl From<(String, String)> for Value {
    fn from((typ, value): (String, String)) -> Self {
        match typ.as_str() {
            "INTEGER" => Value::Integer(value.parse::<i32>().expect("i failed")),
            "STRING" => Value::Str(value),
            "POINT" => {
                let mut nums = value
                    .split(",")
                    .map(|n| f32::from_str(n).expect("i failed"));
                let x = nums.next().expect("i failed");
                let y = nums.next().expect("i failed");
                Value::Point { x, y }
            }
            "RECT" => {
                let mut nums = value
                    .split(",")
                    .map(|n| i32::from_str(n).expect("i failed"));
                let x = nums.next().expect("i failed");
                let y = nums.next().expect("i failed");
                let width = nums.next().expect("i failed");
                let height = nums.next().expect("i failed");
                Value::Rect {
                    x,
                    y,
                    width,
                    height,
                }
            }
            "COLOR" => {
                let mut nums = value
                    .split(",")
                    .map(|n| i32::from_str(n).expect("i failed"));
                let a = nums.next().expect("i failed");
                let r = nums.next().expect("i failed");
                let g = nums.next().expect("i failed");
                let b = nums.next().expect("i failed");
                Value::Color { r, g, b, a }
            }
            _ => {
                panic!("invalid type: {}", typ);
            }
        }
    }
}

impl Element {
    pub fn add_entry(&mut self, key: String, val: Value) {
        self.entries.insert(key, val);
    }
}

#[derive(Debug)]
pub enum Case {
    Start,
    End,
    Section(String),
    Struct(String, String),
    StructVal(String, String, String),
    Unknown,
}

impl From<&String> for Case {
    fn from(line: &String) -> Self {
        Self::from(line.as_str())
    }
}

lazy_static! {
    static ref SECTION_REGEX: Regex =
        Regex::new(r#"^Section = (?P<name>\w+),"(\d,)*\d","\d"$"#).expect("i failed");
    static ref STRUCT_NAME_REGEX: Regex =
        Regex::new(r#"^(?P<name>[a-zA-Z0-9_]+):(?P<type>[a-zA-Z0-9_]+)$"#).expect("i failed");
    /// The trailing `\.?` is not cosmetic: `ifallianceguild.txt:78` ships
    /// `SubSection=STRING,"".` — a stray period after the closing quote.
    /// Anchored on `"$` that line does not match at all, and the old parser
    /// then panicked on it.
    static ref STRUCT_VAL_REGEX: Regex =
        Regex::new(r#"^(?P<key>[a-zA-Z0-9_]+)=(?P<type>[a-zA-Z0-9_]+),"(?P<value>.*)"\.?$"#)
            .expect("STRUCT_VAL_REGEX is a compile-time constant pattern");
}

impl From<&str> for Case {
    fn from(line: &str) -> Self {
        match line {
            "{" => Case::Start,
            "}" => Case::End,
            _ => {
                if SECTION_REGEX.is_match(line) {
                    let capt = SECTION_REGEX.captures(line).expect("i failed");
                    let name = capt.name("name").expect("i failed").as_str();
                    Case::Section(String::from(name))
                } else if STRUCT_NAME_REGEX.is_match(line) {
                    let capt = STRUCT_NAME_REGEX.captures(line).expect("i failed");
                    let name = capt.name("name").expect("i failed").as_str();
                    let ty = capt.name("type").expect("i failed").as_str();
                    Case::Struct(String::from(name), String::from(ty))
                } else if STRUCT_VAL_REGEX.is_match(line) {
                    let capt = STRUCT_VAL_REGEX.captures(line).expect("i failed");
                    let key = capt.name("key").expect("i failed").as_str();
                    let ty = capt.name("type").expect("i failed").as_str();
                    let value = capt.name("value").expect("i failed").as_str();
                    Case::StructVal(String::from(key), String::from(ty), String::from(value))
                } else {
                    Case::Unknown
                }
            }
        }
    }
}

impl From<&Value> for Rect {
    fn from(value: &Value) -> Self {
        match value {
            Value::Rect {
                x,
                y,
                width,
                height,
            } => Rect::new(
                *x as f32,
                *y as f32,
                (*x + *width) as f32,
                (*y + *height) as f32,
            ),
            other => {
                warn!("resinfo: {other:?} is not a rect; using an empty one");
                Rect::default()
            }
        }
    }
}

impl From<&Value> for Color {
    fn from(value: &Value) -> Self {
        match value {
            Value::Color { a, r, g, b } => {
                let red: f32 = *r as f32 / 0xff as f32;
                let green: f32 = *g as f32 / 0xff as f32;
                let blue: f32 = *b as f32 / 0xff as f32;
                let alpha: f32 = *a as f32 / 0xff as f32;
                Color::Srgba(Srgba {
                    red,
                    green,
                    blue,
                    alpha,
                })
            }
            other => {
                warn!("resinfo: {other:?} is not a colour; using the default");
                Color::default()
            }
        }
    }
}

/// The `HAlign`/`VAlign` arm a resinfo value selects: 0 = start, 1 = centre,
/// 2 = end.
///
/// Idea: this is the layer the loader's "malformed input is skipped, not fatal"
/// policy did **not** reach. `resinfo/ifnewitemmallmessagebox.txt:868` ships
/// `HAlign=INTEGER,"1900"` — the corpus' single out-of-range value — and it is
/// *well-formed*, so the parser correctly keeps it and this conversion used to
/// answer it with `panic!`. A key whose value we cannot read is a data problem;
/// it falls back to arm 0 and says so, naming the key and the value, because a
/// silent fallback is how a wrong layout becomes someone else's mystery.
///
/// A missing key lands here too (`None`), which is what retired the sixteen
/// `expect("i failed")` calls: they neither named the key nor survived it.
fn align_arm(key: &str, value: Option<&Value>) -> u8 {
    match value {
        Some(Value::Integer(i @ 0..=2)) => *i as u8,
        Some(Value::Integer(i)) => {
            warn!("resinfo: {key}={i} is outside 0..=2; using 0 (see ifnewitemmallmessagebox.txt:868)");
            0
        }
        Some(other) => {
            warn!("resinfo: {key} is {other:?}, not an integer align value; using 0");
            0
        }
        None => {
            warn!("resinfo: block has no {key}; using 0");
            0
        }
    }
}

/// Read `key`, naming it if it is absent.
///
/// The sixteen call sites used to be `fields.get("…").expect("i failed")` —
/// one message for sixteen different failures, on user-supplied PK2 data, one
/// file away from a loader that documents the opposite policy. A missing key
/// now warns with its own name and the field takes its type default.
fn field<'a>(fields: &'a HashMap<String, Value>, key: &str) -> Option<&'a Value> {
    let value = fields.get(key);
    if value.is_none() {
        warn!("resinfo: block has no {key}; using the type default");
    }
    value
}

impl From<&Value> for Vec2 {
    fn from(value: &Value) -> Self {
        match value {
            Value::Point { x, y } => Vec2::new(*x as f32, *y as f32),
            other => {
                warn!("resinfo: {other:?} is not a point; using (0, 0)");
                Vec2::ZERO
            }
        }
    }
}

impl From<&Value> for i32 {
    fn from(value: &Value) -> Self {
        match value {
            Value::Integer(i) => *i,
            other => {
                warn!("resinfo: {other:?} is not an integer; using 0");
                0
            }
        }
    }
}

impl From<&Value> for String {
    fn from(value: &Value) -> Self {
        match value {
            Value::Str(s) => s.clone(),
            other => {
                warn!("resinfo: {other:?} is not a string; using an empty one");
                String::new()
            }
        }
    }
}

impl From<&Value> for Option<PathBuf> {
    fn from(value: &Value) -> Self {
        match value {
            Value::Str(s) => {
                if s.is_empty() {
                    return None;
                }
                let s = s.replace(r"\\", MAIN_SEPARATOR_STR);
                return Some(PathBuf::from(s));
            }
            other => {
                warn!("resinfo: {other:?} is not a path; using none");
                None
            }
        }
    }
}

/// The original's five text slots as pixel sizes. The ladder itself is data, in
/// points: `Media/server_dep/silkroad/event/event_interface.txt:2` reads
/// `//titlefont : 0 = "9", 1 = "8", 2 = "12", 3 = "11", 4 = "15"`. Points to
/// pixels at 96 dpi is x 4/3, so the five slots are 12 / 10.67 / 16 / 14.67 / 20
/// px.
///
/// **The rounding of the two fractional slots is the original's, not ours.**
/// Its font-slot constructor hands the point sizes to `CreateFontIndirectA` as
/// `lfHeight = -MulDiv(pt, 96, 72)`, and `MulDiv` rounds to nearest — so 8 pt
/// becomes 11 and 11 pt becomes 15 in the client itself. A negative `lfHeight`
/// asks GDI for a character height, which for a TrueType face is the em size,
/// which is what `FontSize::Px` means: the numbers transfer 1:1. (Decompile
/// evidence recorded on the `port/07-hud-frame` lane, `1a19b65dc`; an earlier
/// version of this comment called the rounding ours, which understated the
/// source.)
const FONT_INDEX_PX: [f32; 5] = [12.0, 11.0, 16.0, 15.0, 20.0];

/// Pixel font size of a control whose resinfo block carries
/// `FontIndex=font_index` — the one place that turns that attribute into a
/// number, for the intro, the HUD and the options window alike.
///
/// The size is unscaled: a caller that places its rects at the data's own pixel
/// values wants its text at the same 1:1 factor.
///
/// # Out of range: slot 0, not "the biggest slot"
///
/// An index above 4 names a slot the client never builds, so *some* rule is
/// needed and the data decides which. Measured over the player's own corpus
/// (`Media/resinfo/`, 247 files, 3740 `FontIndex=` attributes):
///
/// * The ladder is **not ordered**: 9, 8, 12, 11, 15 pt. A higher index is not
///   a bigger font, so clamping an out-of-range index to the largest entry
///   reads the index as a magnitude, which the ladder itself contradicts.
/// * Out-of-range indices exist at exactly **three** sites in the whole corpus,
///   all `FontIndex=7` in `resinfo/ifchatbubblewindow.txt:12,31,50`.
/// * The text those three sit among is slot 0: the nine chat trees carry 107
///   `FontIndex` attributes — these three sevens and 104 zeros.
///
/// So slot 0 is the neighbouring authored size at the only place the case can
/// occur, and it is the one choice that does not invent a size ordering. What
/// the original does with index 7 is unknown, so this is a reasoned openroad
/// rule, not a transcribed one.
/// # One-line boxes: pin the line box to the font size
///
/// A resinfo text control is usually authored **exactly one line tall** — the
/// corpus is full of `...x12` rects for a slot-0 (12 px) text, and the original
/// filled them with a GDI font whose cell *is* that 12 px. Bevy's default line
/// box is `LineHeight::RelativeToFont(1.2)`, so the same text lays out 14.4 px
/// tall in a 12 px node and sits low in its box, with the descender over the
/// edge.
///
/// The fix is a relation, not a number: where the authored rect is one line,
/// the site pins `LineHeight::Px(…)` to the **same expression** it passes to
/// `FontSize::Px(…)`, so the glyph box is the authored box. Measured one-line
/// rects behind this: `326x12`, `44x12`, `156x12`, `198x12`, `48x12`,
/// `124x12`, `47x12`, `104x12` (slot 0 or 1 = 12 or 11 px).
///
/// **Not** for multi-line text: `300x48` (the alchemy description, three lines)
/// and the camera pane's stacked caption keep 1.2, where the extra fifth is the
/// gap that stops the lines from touching.
pub const fn font_index_px(font_index: usize) -> f32 {
    if font_index >= FONT_INDEX_PX.len() {
        return FONT_INDEX_PX[0];
    }
    FONT_INDEX_PX[font_index]
}

pub struct Properties {
    pub id: i32,
    pub client_rect: Rect,
    pub color: Color,
    pub ddj: Option<PathBuf>,
    pub font_color: Color,
    pub font_index: i32,
    pub horizontal_align: JustifyContent,
    pub vertical_align: AlignSelf,
    pub rect: Rect,
    pub style: i32, // what does this described? is it actually set? seems like it has values 0,1,4
    pub sub_section: String,
    pub text: String,
    pub uv_lb: Vec2,
    pub uv_lt: Vec2,
    pub uv_rb: Vec2,
    pub uv_rt: Vec2,
}

/// Build the typed view of one block.
///
/// **This cannot fail on data.** It reads user-supplied PK2 text, so every
/// field either converts or falls back to its type default with a warning that
/// names the key — matching the policy the loader one file over already states
/// (`loader.rs:104-107`, "malformed input is skipped, not fatal"). Before this,
/// the two adjacent layers held opposite policies and the noisier one was the
/// one users hit.
impl From<&HashMap<String, Value>> for Properties {
    fn from(fields: &HashMap<String, Value>) -> Self {
        let read = |key: &str| field(fields, key);
        Self {
            id: read("ID").map(Into::into).unwrap_or_default(),
            client_rect: read("ClientRect").map(Into::into).unwrap_or_default(),
            color: read("Color").map(Into::into).unwrap_or_default(),
            ddj: read("DDJ").map(Into::into).unwrap_or_default(),
            font_color: read("FontColor").map(Into::into).unwrap_or_default(),
            font_index: read("FontIndex").map(Into::into).unwrap_or_default(),
            horizontal_align: match align_arm("HAlign", fields.get("HAlign")) {
                1 => JustifyContent::Center,
                2 => JustifyContent::FlexEnd,
                _ => JustifyContent::FlexStart,
            },
            vertical_align: match align_arm("VAlign", fields.get("VAlign")) {
                1 => AlignSelf::Center,
                2 => AlignSelf::FlexEnd,
                _ => AlignSelf::FlexStart,
            },
            rect: read("Rect").map(Into::into).unwrap_or_default(),
            style: read("Style").map(Into::into).unwrap_or_default(),
            sub_section: read("SubSection").map(Into::into).unwrap_or_default(),
            text: read("Text").map(Into::into).unwrap_or_default(),
            uv_lb: read("UV_LB").map(Into::into).unwrap_or_default(),
            uv_lt: read("UV_LT").map(Into::into).unwrap_or_default(),
            uv_rb: read("UV_RB").map(Into::into).unwrap_or_default(),
            uv_rt: read("UV_RT").map(Into::into).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The ladder is the data's, and it is **not ordered** — 9, 8, 12, 11, 15 pt
    /// become 12, 11, 16, 15, 20 px. An out-of-range index therefore cannot be
    /// clamped to the largest entry without reading the index as a magnitude;
    /// it resolves to slot 0, the size its only real occurrence sits among
    /// (`font_index_px`).
    #[test]
    fn the_font_ladder_is_the_data_s_and_out_of_range_reads_slot_0() {
        assert_eq!(
            [
                font_index_px(0),
                font_index_px(1),
                font_index_px(2),
                font_index_px(3),
                font_index_px(4)
            ],
            [12.0, 11.0, 16.0, 15.0, 20.0]
        );
        // not monotonic: slot 1 is smaller than slot 0
        assert!(font_index_px(1) < font_index_px(0));
        // the only out-of-range index the corpus actually contains
        assert_eq!(font_index_px(7), font_index_px(0));
        assert_ne!(font_index_px(7), font_index_px(4), "not the biggest slot");
    }

    /// The attribute arrives as text and is read by key, so the path a pane
    /// really walks is `FontIndex=INTEGER,"n"` -> [`Value::Integer`] ->
    /// [`Properties::font_index`] -> pixels. Both indices are verbatim from the
    /// corpus, and the non-zero one is the half that matters: slot 0 is also the
    /// type default, so only index 2 proves the attribute is read at all.
    #[test]
    fn a_font_index_attribute_resolves_to_its_pixel_size() {
        let mut fields = complete_block();
        // `GDR_OPTION_AUD_ST_BGM` (`resinfo/ifoption_audio.txt:221`)
        fields.insert(
            "FontIndex".into(),
            Value::parse("INTEGER", "0", 221).unwrap(),
        );
        let props = Properties::from(&fields);
        assert_eq!(props.font_index, 0);
        assert_eq!(font_index_px(props.font_index as usize), 12.0);

        // `GDR_BTN_SACCEPT` (`resinfo/pstitle_europe.txt:545`)
        fields.insert(
            "FontIndex".into(),
            Value::parse("INTEGER", "2", 545).unwrap(),
        );
        let props = Properties::from(&fields);
        assert_eq!(props.font_index, 2);
        assert_eq!(font_index_px(props.font_index as usize), 16.0);
    }

    /// The corpus' one out-of-range align value, verbatim from
    /// `resinfo/ifnewitemmallmessagebox.txt:868`. It is well-formed — the
    /// parser is right to keep it — and this conversion used to answer it with
    /// a panic, so a single line in 247 user-supplied files could take the
    /// client down the day `Properties` gained its first consumer.
    #[test]
    fn the_corpus_out_of_range_halign_yields_a_usable_properties() {
        let mut fields = complete_block();
        fields.insert(
            "HAlign".into(),
            Value::parse("INTEGER", "1900", 868).unwrap(),
        );

        let props = Properties::from(&fields);

        assert_eq!(props.horizontal_align, JustifyContent::FlexStart);
        // the rest of the block still reads normally
        assert_eq!(props.id, 7);
        assert_eq!(props.text, "ok");
        assert_eq!(props.vertical_align, AlignSelf::Center);
    }

    /// A missing key is the other half of the same defect: the sixteen
    /// `expect("i failed")` calls neither named the key nor survived it, and a
    /// parser that skips a malformed line hands this layer exactly that.
    #[test]
    fn a_missing_key_falls_back_instead_of_killing_the_client() {
        let mut fields = complete_block();
        fields.remove("Rect");
        fields.remove("Text");
        fields.remove("HAlign");

        let props = Properties::from(&fields);

        assert_eq!(props.rect, Rect::default());
        assert_eq!(props.text, "");
        assert_eq!(props.horizontal_align, JustifyContent::FlexStart);
        assert_eq!(props.id, 7);
    }

    /// A well-formed value of the wrong *type* is the third arm — the parser
    /// types by the declared token, so `Rect=STRING,"..."` reaches here intact.
    #[test]
    fn a_wrongly_typed_value_falls_back_instead_of_killing_the_client() {
        let mut fields = complete_block();
        fields.insert("Rect".into(), Value::Str("not a rect".into()));
        fields.insert("HAlign".into(), Value::Str("centre".into()));

        let props = Properties::from(&fields);

        assert_eq!(props.rect, Rect::default());
        assert_eq!(props.horizontal_align, JustifyContent::FlexStart);
    }

    /// A block carrying every key the typed view reads, all in range.
    fn complete_block() -> HashMap<String, Value> {
        let v = |typ: &str, raw: &str| Value::parse(typ, raw, 1).unwrap();
        [
            ("ID", v("INTEGER", "7")),
            ("ClientRect", v("RECT", "0,0,10,10")),
            ("Color", v("COLOR", "255,1,2,3")),
            ("DDJ", v("STRING", "")),
            ("FontColor", v("COLOR", "255,4,5,6")),
            ("FontIndex", v("INTEGER", "7")),
            ("HAlign", v("INTEGER", "0")),
            ("VAlign", v("INTEGER", "1")),
            ("Rect", v("RECT", "1,2,3,4")),
            ("Style", v("INTEGER", "0")),
            ("SubSection", v("STRING", "")),
            ("Text", v("STRING", "ok")),
            ("UV_LB", v("POINT", "0,1")),
            ("UV_LT", v("POINT", "0,0")),
            ("UV_RB", v("POINT", "1,1")),
            ("UV_RT", v("POINT", "1,0")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }
}
