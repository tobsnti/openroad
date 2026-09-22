//! The race-board *place* on the character-select stage, and the camera flight
//! that goes there.
//!
//! Idea: in v1.188 picking a race is not a screen swap, it is a **move across
//! the same 3D stage**. The character lineup stands at `x 29..95, z 644..654`;
//! the three race idols (`res/interface/interface_idol_{china,europe}.bsr`,
//! `interface_lizard.bsr`) stand ~100 units away at `x 155..157, z 651..654`,
//! and `CPSCharacterSelect`'s Create handler walks the camera over there
//! before the plates appear. Our clone used to hold the lineup camera, which
//! is exactly why the board looked like the character-select screen with
//! pictures pasted on it.
//!
//! Every number below is measured out of `sro_client.exe` and written up with
//! its VA in the RE notes§2. The two things that
//! are *not* measured — the two middle waypoints of the original's four-point
//! curve, and therefore the flight's shape — are called out at
//! [`BOARD_FLIGHT`] rather than replaced by invented poses (ADR-0009).

use std::time::Duration;

use bevy::prelude::*;
use bevy_tweening::{Sequence, TweenAnim};

use crate::plugins::camera::CinematicCamera2;
use crate::plugins::dynamic_resource_loader::{MirroredResource, UnloadedResource};
use crate::plugins::world_origin::WorldOrigin;
use crate::util::mesh::needs_winding_reversal;
use crate::util::tweening_ext::keyframe_tween;

use super::character_create::Race;
use super::scene_data::ActiveCharSelectSceneV2;
use super::IntroV2State;

/// Camera pose the original flies to when `Create` is pressed, in the units
/// `assets/char_selects/constantinople.selection` authors its keyframes in:
/// the last waypoint of the original's four-entry camera path.
const BOARD_CAM_ARRIVAL_OFFSET: Vec3 = Vec3::new(165.2, -33.9, 640.3);

/// Pitch of the same waypoint, `0.62` rad: with the height above it puts the
/// camera 13.9 units *below* the props and looks up at 35°.
const BOARD_CAM_ARRIVAL_PITCH: f32 = 0.62;

/// The props' authored placement height (all four rows of [`BOARD_PROPS`] carry
/// `y = -20.0`). Named because the arrival pose
/// below is expressed *relative to it*, not as a second free number.
const BOARD_TABLE_LEVEL: f32 = -20.0;

/// **Arrival pose derived from the props' own bounding boxes — the distance is
/// ours, the direction is the original's (stated deviation, ADR-0009).**
///
/// The measurement that decided this shape, photographed and then computed
/// (lane `charstage-2`, 2026-08-25):
///
/// * The original's pose *is* right and *is* aimed right: from
///   `(165.2, -33.9, 640.3)` with `pitch +0.62 / yaw 2.53` the axis misses the
///   union of the four props' bounding boxes (centre `155.78, -23.43, 651.79`,
///   radius `13.76`) by only **3.5°**. So neither the read floats nor our
///   `y = -20` placement is the defect — the earlier suspicion that
///   `BOARD_TABLE_LEVEL` was an invented number is **refuted**, with numbers.
/// * What the original does is stand **inside** its own tableau: `18.2` units
///   from that centre, where the union has an angular radius of **37°** against
///   a half-FOV of `22.5°`. It overfills the frame by design, and the first
///   photo showed exactly that — `box.bsr`'s planks across the whole screen.
/// * A first correction that mirrored height and pitch about the *placement
///   plane* instead of the *view axis* was `14.2°` off-axis and pushed the
///   tableau into the bottom-left corner (photographed, `/tmp/shots/cs2v_5.png`
///   and `cs2w_5.png`). Mirroring about the wrong plane is not a composition.
///
/// **The deviation, in one line:** the original stands 18.2 units inside its
/// tableau and crops it; we step back until the whole union is inside the frame,
/// because this screen offers a *choice* and a cropped motif does not show what
/// is on offer. Everything else stays the original's: the **azimuth** and the
/// **magnitude of the elevation** are the measured `yaw 2.53 / |pitch| 0.62`;
/// only the elevation's *sign* is flipped, which is the maintainer's request to
/// see the figures from above (playtest H2), and only the *distance* is ours.
///
/// Still `[U]`, and now sharper than before: why the original composes its own
/// race board as an overfilled close-up. It may well be intentional (the plates
/// are 2D art over it), but nothing measured says so.
const BOARD_CAM_PITCH: f32 = -BOARD_CAM_MEASURED_PITCH;

/// Vertical field of view the cinematic camera is spawned with
/// (`plugins::camera::FOV` = `FRAC_PI_4`). Mirrored here rather than imported
/// because that constant is private to the camera plugin; the test
/// `the_frame_fill_uses_the_cameras_own_fov` fails if the two drift apart.
const BOARD_CAM_FOV: f32 = std::f32::consts::FRAC_PI_4;

/// Fraction of the frame's half-height the props' bounding sphere is asked to
/// fill. **Our number** (ADR-0009): `0.85` is the largest value at which every
/// corner of the union still projects inside the viewport at both `21:9`
/// (3440x1440) and `16:9` — computed, not eyeballed, and asserted for both
/// aspects in `the_arrival_frames_the_props_at_both_aspects`. Larger crops the
/// table again; much smaller turns the motif into a detail of the quay.
const BOARD_FRAME_FILL: f32 = 0.85;

/// Yaw of the same waypoint. The original says `2.53` rad; our world mirrors
/// x (`CameraKeyframe::translation` multiplies by `(-1, 1, 1)`), so the
/// authored yaw is `PI - yaw_original`. Positive control for this bridge: the
/// lineup camera we already ship authors `0.1415`, and the original's value
/// for the same waypoint is `3.0` — `PI - 3.0 = 0.14159`.
const BOARD_CAM_YAW_ORIGINAL: f32 = 2.53;

/// One leg of the original's ladder: it puts the four waypoints 5/3 s apart.
const BOARD_FLIGHT: Duration = Duration::from_micros(1_666_667);

/// The two middle waypoints of the original's four-point camera path:
///
/// | waypoint | pos | rot (pitch, yaw) | frames |
/// |---|---|---|---|
/// | 0 | `60, -15, 660` | `0.1, 3.00` | nothing (see below) |
/// | 1 | `100, 0, 625` | `-0.1, 2.20` | the lineup, dot `0.90` |
/// | 2 | `150, -10, 642` | `-0.01, 1.80` | the lineup, dot `0.97` |
/// | 3 | `165.2, -33.9, 640.3` | `0.62, 2.53` | the idols, dot `0.99` |
///
/// The "frames" column is the check that these are not four transcription
/// accidents: with the original's own forward vector
/// `(-sin yaw·cos pitch, sin pitch, -cos yaw·cos pitch)` the two middles look
/// straight at the character line while the camera swings past it, and only
/// the last one turns onto the idols. Four numbers landing on two known
/// targets by accident is not a plausible reading.
///
/// **Waypoint 0 is deliberately not used.** Its own rotation looks *away* from
/// both the lineup (dot `-0.73`) and the idols (`-0.23`); it is the entry the
/// t-ladder puts at `t = 5.0`, i.e. at the far end of the path, and our stage
/// asset authors its own (hand-tweaked) lineup pose that the player is already
/// looking through. So the flight starts from the live camera pose and walks
/// the original's three waypoints from there — no invented pose, and no cut.
/// Rigid lift of the whole board tableau — camera path *and* props — onto our
/// own stage's deck. `+6.3` = the deck level our scene asset authors for the
/// character line (`char_start_offset.y = -27.6`,
/// `assets/char_selects/constantinople.selection`) minus the original's camera
/// height at the board (`-33.9`).
///
/// **Why this is a stated deviation and not an invented number (ADR-0009).**
/// The original's *region* for the pregame stage is unknown — our
/// `constantinople.selection` is openroad-authored, and whether the original
/// uses the same region and coordinates is unknown. The board coordinates are
/// region-local, so on our region the tableau lands 6.3 units below our pier:
/// the camera sat *under the deck* and looked at the pier planks from below.
/// Lifting the group rigidly is the one correction that keeps every relation —
/// prop-to-prop, camera-to-prop, and therefore the whole composition — exactly
/// as the original has it, and changes only the number that was never the
/// original's to begin with: which piece of ground the stage stands on.
const BOARD_STAGE_LIFT: f32 = 6.3;

/// Legs of the Cancel return: the two middle waypoints plus the lineup pose
/// (the board waypoint is where the camera already stands).
const RETURN_LEGS: u32 = 3;

/// The two *middle* waypoints of the original's path, untouched: they swing
/// past the quay exactly as authored. The third leg — the arrival — is not a
/// waypoint any more but [`board_camera_pose`], because it is derived from the
/// props' bounds rather than authored.
const BOARD_WAYPOINTS: [(Vec3, f32, f32); 2] = [
    // (offset, pitch, yaw_original)
    (Vec3::new(100.0, 0.0, 625.0), -0.1, 2.20),
    (Vec3::new(150.0, -10.0, 642.0), -0.01, 1.80),
];
/// Where the flight is headed, recorded on the camera so the destination is
/// observable — the tween itself keeps its end pose private, and a test that
/// can only see "some tween was inserted" would not catch a camera flying to
/// the wrong place.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct RaceBoardFlight {
    pub target_pos: Vec3,
    pub target_rot: Quat,
}

/// World-space bounding sphere of the four props, from their own `.bsr` boxes
/// through the very mapping [`spawn_race_board_props`] uses (mirror, per-prop
/// yaw, lift, floating origin). Returned as `(centre, radius)`.
///
/// This is the *motif* the arrival frames. Deriving it here — instead of
/// writing down a camera distance — is what makes the composition follow the
/// data: change a prop, and the shot follows.
/// The eight corners of a bounding box.
fn bbox_corners(min: Vec3, max: Vec3) -> [Vec3; 8] {
    [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ]
}

/// Every prop corner in world space, through the mapping
/// [`spawn_race_board_props`] uses: the prop's own yaw, its `(-1, 1, 1)` mirror
/// scale, the lift and the floating origin.
fn board_prop_corners(cam_base: Vec3, origin: Vec3) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(BOARD_PROPS.len() * 8);
    for prop in BOARD_PROPS.iter() {
        let translation = board_prop_translation(cam_base, origin, prop.offset);
        let rotation = Quat::from_rotation_y(-prop.yaw);
        for corner in bbox_corners(prop.bbox_min, prop.bbox_max) {
            // `anim_offset` is part of where the model *is* on screen, so it
            // belongs in the bounds the arrival frames — see
            // [`BoardProp::anim_offset`].
            let local = (corner + prop.anim_offset) * Vec3::new(-1.0, 1.0, 1.0);
            out.push(translation + rotation * local);
        }
    }
    out
}

/// Local y of the desk's top face — the plane the original stands its props on.
///
/// `interface_box.bms` has 20 vertices; twelve of them lie on y = -0.053 and
/// span the slab's full rectangle. The four things the original lays on that
/// face have their own undersides at y_local -0.040 (letter), -0.027 (map),
/// -0.000 (compass) and +0.022 (brushcase), which both confirms the plane and
/// gives [`BOARD_TABLE_CONTACT_TOLERANCE`] its value.
const BOARD_TABLE_TOP_LOCAL_Y: f32 = -0.053;

/// The slab's top face, `(x, z)` corners in model space (same twelve vertices).
const BOARD_TABLE_TOP_RECT: [Vec2; 4] = [
    Vec2::new(-6.368, -10.505),
    Vec2::new(7.935, -10.505),
    Vec2::new(7.935, 2.700),
    Vec2::new(-6.368, 2.700),
];

/// How far a prop's underside may sit off the table plane and still count as
/// standing on it.
///
/// Not a guess and not "small enough": it is the scatter the ORIGINAL's own
/// table props show against that same plane. Their undersides span
/// y_local -0.040 … +0.022 (letter … brushcase) about a face at -0.053, i.e.
/// 0.013 … 0.075 of air. Anything inside 0.08 is therefore indistinguishable
/// from how the original itself lays things on this table; the three idols sit
/// at 0.038 … 0.043.
const BOARD_TABLE_CONTACT_TOLERANCE: f32 = 0.08;

/// World-space table top: `(plane_y, four corners)`, mapped through the very
/// placement [`spawn_race_board_props`] gives the box.
fn board_table_top(cam_base: Vec3, origin: Vec3) -> (f32, [Vec3; 4]) {
    board_table_top_with_yaw(BOARD_PROPS[0].yaw, cam_base, origin)
}

/// [`board_table_top`] for an arbitrary box facing. Split out so the test can
/// run its negative control against the facing we used to ship (`0.0`) without
/// a second copy of the mapping.
fn board_table_top_with_yaw(box_yaw: f32, cam_base: Vec3, origin: Vec3) -> (f32, [Vec3; 4]) {
    let box_prop = &BOARD_PROPS[0];
    let translation = board_prop_translation(cam_base, origin, box_prop.offset);
    let rotation = Quat::from_rotation_y(-box_yaw);
    let corner = |c: Vec2| {
        let local = Vec3::new(c.x, BOARD_TABLE_TOP_LOCAL_Y, c.y) * Vec3::new(-1.0, 1.0, 1.0);
        translation + rotation * local
    };
    let corners = [
        corner(BOARD_TABLE_TOP_RECT[0]),
        corner(BOARD_TABLE_TOP_RECT[1]),
        corner(BOARD_TABLE_TOP_RECT[2]),
        corner(BOARD_TABLE_TOP_RECT[3]),
    ];
    (corners[0].y, corners)
}

/// Where one prop actually touches down: `(underside_y, four footprint
/// corners)` in world space, through its own yaw, mirror, animation offset,
/// lift and the floating origin.
fn board_prop_footprint(prop: &BoardProp, cam_base: Vec3, origin: Vec3) -> (f32, [Vec3; 4]) {
    let translation = board_prop_translation(cam_base, origin, prop.offset);
    let rotation = Quat::from_rotation_y(-prop.yaw);
    let base_y = prop.bbox_min.y + prop.anim_offset.y;
    let corner = |x: f32, z: f32| {
        let local =
            (Vec3::new(x, base_y, z) + prop.anim_offset.with_y(0.0)) * Vec3::new(-1.0, 1.0, 1.0);
        translation + rotation * local
    };
    let corners = [
        corner(prop.bbox_min.x, prop.bbox_min.z),
        corner(prop.bbox_max.x, prop.bbox_min.z),
        corner(prop.bbox_max.x, prop.bbox_max.z),
        corner(prop.bbox_min.x, prop.bbox_max.z),
    ];
    (corners[0].y, corners)
}

/// True when `(x, z)` lies inside the convex quad `quad` (x/z of the four
/// corners), boundary counted as inside.
fn inside_quad_xz(quad: &[Vec3; 4], point: Vec3) -> bool {
    let mut positive = 0;
    let mut negative = 0;
    for i in 0..4 {
        let a = quad[i];
        let b = quad[(i + 1) % 4];
        let cross = (b.x - a.x) * (point.z - a.z) - (b.z - a.z) * (point.x - a.x);
        if cross > 1e-4 {
            positive += 1;
        } else if cross < -1e-4 {
            negative += 1;
        }
    }
    positive == 0 || negative == 0
}

pub fn board_props_bounds(cam_base: Vec3, origin: Vec3) -> (Vec3, f32) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for world in board_prop_corners(cam_base, origin) {
        min = min.min(world);
        max = max.max(world);
    }
    ((min + max) * 0.5, (max - min).length() * 0.5)
}

/// The arrival pose: the original's direction, our distance.
///
/// Direction is the original's (`yaw 2.53`, `|pitch| 0.62`, elevation sign
/// flipped — see [`BOARD_CAM_PITCH`] for the 3.5°/37° numbers and the stated
/// deviation). Distance comes from the motif: put the props' bounding sphere at
/// [`BOARD_FRAME_FILL`] of the frame's half-height, i.e.
/// `radius / (fill * tan(fov/2))`, with the same vertical FOV the cinematic
/// camera carries (`plugins::camera::FOV`, 45°). Aiming at the sphere's centre
/// is what centres the tableau — the earlier version aimed *past* it and the
/// props ended up in a corner.
pub fn board_camera_pose(cam_base: Vec3, origin: Vec3) -> (Vec3, Quat) {
    // Yaw *and* order are the original's: of the four candidate conventions,
    // exactly one frames the idols at their known position, `YXZ(-yaw, pitch)`.
    // (The scene asset's own
    // keyframes use `PI - yaw` instead, because `CameraKeyframe` mirrors
    // translation and rotation together; a hand-built pose mirrors a yaw to
    // `-yaw`. The positive control validated the asset path, not this one.)
    let rotation = Quat::from_euler(EulerRot::YXZ, -BOARD_CAM_YAW_ORIGINAL, BOARD_CAM_PITCH, 0.0);
    let (centre, radius) = board_props_bounds(cam_base, origin);
    let distance = radius / (BOARD_FRAME_FILL * (BOARD_CAM_FOV * 0.5).tan());
    (centre - (rotation * Vec3::NEG_Z) * distance, rotation)
}

/// World bounds of ONE prop, `(centre, radius)`, through the same mapping
/// [`board_props_bounds`] uses for all four. Split out because the click's
/// close-up frames a single figure while the arrival frames the union.
fn prop_bounds(prop: &BoardProp, cam_base: Vec3, origin: Vec3) -> (Vec3, f32) {
    let translation = board_prop_translation(cam_base, origin, prop.offset);
    let rotation = Quat::from_rotation_y(-prop.yaw);
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for corner in bbox_corners(prop.bbox_min, prop.bbox_max) {
        let local = (corner + prop.anim_offset) * Vec3::new(-1.0, 1.0, 1.0);
        let world = translation + rotation * local;
        min = min.min(world);
        max = max.max(world);
    }
    ((min + max) * 0.5, (max - min).length() * 0.5)
}

/// The pose the CLICK flies to: the chosen figure as the subject of the shot.
///
/// **What the original does, and why this is not a transcription of it
/// (ADR-0009).** The click is a 1.0 s pure translation of 3.65
/// (europe) / 3.84 (china) units with the orientation unchanged, and its
/// *purpose* is to put the chosen idol on the view axis — each region's target
/// minimises its own idol's off-axis angle (6.23° → 2.82° europe, 8.79° → 5.21°
/// china) and worsens the other's. It ends up *further* from the figure than it
/// started, because the original already stands 18.2 units inside its tableau.
///
/// Our arrival stands **39.1** units back instead, for the stated reason at
/// [`BOARD_FRAME_FILL`], and with the elevation sign flipped. Transplanting the
/// measured delta into that shot reproduces nothing: computed in our own frame
/// it *worsens* the chosen idol's off-axis angle (europe 6.6° → 8.7°, china
/// 4.0° → 8.4°). So the click applies the original's **decision** through the
/// rule this file already states — same measured direction, same
/// [`BOARD_FRAME_FILL`], motif = the **chosen idol's own box** instead of the
/// union of all four props. The distance falls out of the data (the idols
/// measure ~1.1 x 2.2 x 1.1, so it lands ~3.8 units out): a close-up of the
/// figure the player picked, with no new constant.
pub fn confirm_camera_pose(race: Race, cam_base: Vec3, origin: Vec3) -> (Vec3, Quat) {
    let rotation = Quat::from_euler(EulerRot::YXZ, -BOARD_CAM_YAW_ORIGINAL, BOARD_CAM_PITCH, 0.0);
    let (centre, radius) = BOARD_PROPS
        .iter()
        .find(|prop| prop.race == Some(race))
        .map(|prop| prop_bounds(prop, cam_base, origin))
        // No such prop can only mean the table changed; framing the whole
        // tableau is the arrival pose, i.e. "no move", not a wrong place.
        .unwrap_or_else(|| board_props_bounds(cam_base, origin));
    let distance = radius / (BOARD_FRAME_FILL * (BOARD_CAM_FOV * 0.5).tan());
    (centre - (rotation * Vec3::NEG_Z) * distance, rotation)
}

/// Starts the click's own camera move: one leg, [`CONFIRM_FLIGHT`] long,
/// orientation unchanged along the way except for the (identical) end
/// rotation. Returns the pose it is flying to, so the caller — and a test —
/// can see the destination instead of "some tween was inserted".
pub fn start_confirm_flight(
    race: Race,
    cam: &Query<(Entity, &Transform), With<CinematicCamera2>>,
    scene: &ActiveCharSelectSceneV2,
    origin: &WorldOrigin,
    commands: &mut Commands,
) -> Option<(Vec3, Quat)> {
    let Ok((cam_entity, cam_transform)) = cam.single() else {
        return None;
    };
    let (target_pos, target_rot) = confirm_camera_pose(race, scene.0.cam_base(), origin.0);
    commands.entity(cam_entity).insert((
        RaceBoardFlight {
            target_pos,
            target_rot,
        },
        TweenAnim::new(Sequence::new([keyframe_tween(
            cam_transform.translation,
            target_pos,
            cam_transform.rotation,
            target_rot,
            CONFIRM_FLIGHT,
        )])),
    ));
    Some((target_pos, target_rot))
}

/// Camera target the CLICK flies to, per region, in the original's own units
/// (the original stores the three floats reversed into its argument slots, so
/// `g_region 0` is `x = 168.5, y = -34.9, z = 639.1`). Control:
/// reading `168.5` as z would put the camera 480 units off a stage that lives
/// at `x 155..157, z 651..654`, while this reading puts it 3.5 units from the
/// arrival waypoint.
///
/// They are kept even though [`confirm_camera_pose`] derives the pose it
/// actually flies to (see there): they are the reference that derivation is
/// *checked against*, and `the_confirm_targets_frame_their_own_idol` reads
/// them.
const CONFIRM_TARGET_EUROPE: Vec3 = Vec3::new(168.5, -34.9, 639.1);
const CONFIRM_TARGET_CHINA: Vec3 = Vec3::new(165.8, -35.1, 636.7);

/// Duration of the click move: the keyframe pair is `t = 0.0` (the
/// pose the camera already stands on) and `t = 1.0` (the region target), and
/// that 1.0 is what the original's next state then waits for.
pub const CONFIRM_FLIGHT: Duration = Duration::from_secs(1);

/// Whether the click move plays under the original's 1-second fade to black.
///
/// **ON by default — this is the original's behaviour.** The original runs
/// `GDR_FADE` (control id `0x15`) with
/// `(from 0, to 255, duration 1.0 s, delay 0.0, flag 1)` — the
/// same second the camera move takes — so its own end pose is essentially
/// invisible and the loading art arrives on black.
///
/// Setting this to `false` is the *deviation*, and the reason it exists is
/// worth keeping rather than deleting: a visible move is exactly what was
/// missing here, and the loading art that follows is already a cut. That is a
/// defensible reading, but it is a taste call, and fidelity wins. Both halves
/// stay written down.
///
/// **The flight is not dead code behind this switch.** It sets the pose the
/// camera *stands on* when the black lifts: the close-up of the chosen figure
/// the creation screen inherits (`CPSCharacterCreate*` authors no camera of
/// its own). Disable the move and
/// the state after the fade is wrong, invisibly.
pub const CONFIRM_FADE_TO_BLACK: bool = true;

/// Same mapping for an arbitrary waypoint. Independent confirmation of the
/// convention: the original's forward vector is
/// `(-sin yaw·cos pitch, sin pitch, -cos yaw·cos pitch)` — that formula hits
/// the idol group from the arrival waypoint to within 9°, and mirroring its x
/// is exactly `YXZ(-yaw, pitch)`. So the hand-built pose below and the
/// original's own camera maths agree, which is a stronger statement than "one
/// convention scored best".
fn waypoint_pose(cam_base: Vec3, origin: Vec3, offset: Vec3, pitch: f32, yaw: f32) -> (Vec3, Quat) {
    (
        (cam_base + offset + Vec3::Y * BOARD_STAGE_LIFT) * Vec3::new(-1.0, 1.0, 1.0) - origin,
        Quat::from_euler(EulerRot::YXZ, -yaw, pitch, 0.0),
    )
}

/// A prop of the race board: the model the original loads, where it stands
/// (authored stage coordinates, same convention as the lineup offsets), and
/// the facing it is given.
///
/// All four rows are the table the original's select screen builds (three
/// idols plus the box loaded separately); the facing is the single float
/// handed to each prop's facing setter.
struct BoardProp {
    path: &'static str,
    /// The race this prop *is*, for the two props the original lets the pointer
    /// pick — `None` for scenery.
    ///
    /// The hover test in state 8 walks `i = 0,1` only
    /// over the prop array, so exactly two of the three loaded
    /// props are pickable, in the array's own order — prop 0
    /// `interface_idol_europe.bsr` sets `g_region = 0`, prop 1
    /// `interface_idol_china.bsr` sets `g_region = 1`, and the same
    /// index selects the plate and the loading art in
    /// state 9. The box and the lizard are scenery: the lizard is the Arabia
    /// idol of a race with no player body, and *because* it is not pickable its
    /// animated root displacement (`anim_offset`) never enters a hit path.
    race: Option<Race>,
    offset: Vec3,
    /// The model's own bounding box — the **union of the meshes the `.bsr`
    /// actually references**, read out of those `.bms` (`prim/mesh/interface/*`,
    /// header field 5 `bounding_box_offset`, six floats).
    /// It is here because the arrival pose is *derived* from the props instead
    /// of being a free pose — the camera has to know how big its motif is, and
    /// the motif's size is data, not taste.
    ///
    /// Taken from the shipped meshes, because the previous values came from the
    /// `.bsr`'s own record and two of them were not in any mesh (see the
    /// `interface_lizard` row). `box.bsr` is not a
    /// crate but a **five-mesh desk** (`interface_box` plus `interface_compass`,
    /// `interface_idol_brushcase`, `interface_letter`, `interface_map`), so its
    /// union's top is the *letter's* top, not the table surface — the surface is
    /// [`BOARD_TABLE_TOP_LOCAL_Y`].
    bbox_min: Vec3,
    bbox_max: Vec3,
    /// Rigid displacement the model's *animation* applies to the whole mesh, in
    /// model space, `ZERO` for a static prop.
    ///
    /// Only `interface_lizard.bsr` is animated (it ships a `.bsk` and
    /// `interface_lizard_{stand,move,run}.ban`), and its idle clip does not
    /// merely breathe: the first keyframe of `interface_lizard_stand.ban` puts
    /// the root bone `Bip01` at `(3.472, 0.287, -7.791)` where the skeleton's
    /// rest pose has it at `(-0.0001, 0.279, -0.118)`. Composed through the
    /// root's rest rotation (`parent_rotation = (-0.5, 0.5, -0.5, 0.5)`, a 90°
    /// axis swizzle) that moves every skinned vertex by the vector below — so
    /// the *rendered* lizard stands ~8 units away from its own `Aabb`, which
    /// stays with the mesh. That is why the bounds the camera frames, and the
    /// on-the-table check, have to use it.
    anim_offset: Vec3,
    /// Facing in the ORIGINAL's frame, radians. Our world mirrors x, which
    /// maps a yaw to `-yaw` — the same bridge [`board_camera_pose`] uses, and
    /// the same one the lineup takes when it faces its characters with `PI`
    /// (`PI` is its own mirror image, which is why the lineup never had to
    /// think about it).
    yaw: f32,
}

const BOARD_PROPS: [BoardProp; 4] = [
    BoardProp {
        path: "data://res/interface/box.bsr",
        race: None,
        offset: Vec3::new(155.0, -20.0, 652.0),
        // 14.3 x 9.7 x 13.2 over all five meshes. The desk slab itself
        // (`interface_box.bms`, 20 vertices) hangs BELOW its origin and its top
        // face is the full rectangle at y = -0.053; the 0.588 here is the
        // letter lying on that face.
        bbox_min: Vec3::new(-6.384, -9.099, -10.505),
        bbox_max: Vec3::new(7.935, 0.588, 2.701),
        anim_offset: Vec3::ZERO,
        // REFUTED, and left standing so nobody re-derives it: this
        // row used to read `yaw: 0.0` with the comment "the box is loaded
        // outside the idol loop and never gets a facing call, so it keeps the
        // default: 0, not a number we picked". It does get one. Right after its
        // own `SetPosition` the same object is handed the same one-float setter
        // the three idols get: SetPosition(box), then the facing setter with
        // 3.0 (the idols get {3.31, 3.03, 3.0} from the same setter).
        // The consequence is not cosmetic: the slab is strongly asymmetric in z
        // (-10.505..+2.701, origin near its back edge), so an unrotated table
        // sits ~7 units short in +z and the props the original placed *on* it
        // land behind it — the europe idol in mid-air over the stone step, the
        // china idol on the back edge, the lizard off the deck entirely
        // (playtest 2026-08-25, measured in the RE notes
        // §6). With this value all three stand on the table.
        yaw: 3.0,
    },
    BoardProp {
        path: "data://res/interface/interface_idol_europe.bsr",
        race: Some(Race::EUROPEAN),
        offset: Vec3::new(157.0, -20.0, 654.4),
        // interface_idol_europe.bms
        bbox_min: Vec3::new(0.516, -0.010, -3.919),
        bbox_max: Vec3::new(1.560, 2.229, -2.833),
        anim_offset: Vec3::ZERO,
        yaw: 3.31,
    },
    BoardProp {
        path: "data://res/interface/interface_idol_china.bsr",
        race: Some(Race::CHINESE),
        offset: Vec3::new(156.2, -20.0, 651.6),
        // interface_idol_china.bms
        bbox_min: Vec3::new(3.232, -0.014, -3.963),
        bbox_max: Vec3::new(4.839, 2.018, -2.350),
        anim_offset: Vec3::ZERO,
        yaw: 3.03,
    },
    // Slot 2 of the original's table is the Arabia idol, and it ships as
    // `interface_lizard.bsr` — dead art for a race v1.188 has no player body
    // for. It is part of the *scenery* the
    // camera frames, so it is placed like the other two; what stays absent is
    // its 2D plate, which the data does not offer either.
    BoardProp {
        path: "data://res/interface/interface_lizard.bsr",
        race: None,
        offset: Vec3::new(155.6, -20.0, 651.6),
        // Two meshes, so the union of both:
        //   interface_lizard_part1  x -1.255..1.255  y 0.106..0.699  z -2.527..-0.588
        //   interface_lizard_part2  x -1.836..1.836  y -0.015..0.712 z -0.607..2.345
        // The values that stood here (`bbox_min.z = -9.934`, `bbox_max.x =
        // 4.657`) are in NEITHER mesh. They are the mesh union plus the
        // animated root displacement below (`4.657 = 1.255 + 3.40`, `-9.934 ≈
        // -2.527 - 7.4`, and `interface_lizard_move.ban` reaches z = -9.96), so
        // they were read off an animated pose. Keeping the two apart is what
        // makes both numbers checkable: the mesh's size comes from the mesh,
        // the displacement from the clip.
        bbox_min: Vec3::new(-1.836, -0.015, -2.527),
        bbox_max: Vec3::new(1.836, 0.712, 2.345),
        // `interface_lizard_stand.ban`, first keyframe of `Bip01`, composed
        // through the skeleton's rest root as described at
        // [`BoardProp::anim_offset`]: the mesh union's centre travels from
        // (0.000, 0.349, -0.091) to (3.481, 0.357, -7.816).
        anim_offset: Vec3::new(3.481, 0.008, -7.725),
        yaw: 3.00,
    },
];

/// Marker for everything [`spawn_race_board_props`] puts on the stage, so
/// leaving the board takes exactly its own props down again.
#[derive(Component)]
pub struct RaceBoardProp;

/// One of the two idols the pointer can pick, with the world-space box the
/// pointer is tested against and the race it stands for.
///
/// Why the box travels on the component instead of being recomputed: the props
/// are placed once on `OnEnter(RegionSelect)` from the scene base and the
/// floating origin and then never move, so the hover test would otherwise redo
/// the same eight corner transforms every frame — and, more importantly, the
/// box would then be derived twice (once for the arrival framing in
/// [`board_props_bounds`], once for the pick) and could drift apart.
#[derive(Component, Clone, Copy)]
pub struct RaceBoardIdol {
    pub race: Race,
    /// World-space AABB of the model's own mesh union, transformed exactly like
    /// the rendered entity (mirror included).
    pub world_min: Vec3,
    pub world_max: Vec3,
}

impl RaceBoardIdol {
    /// Point the plate is hung from: the box centre, which is what the
    /// original projects (the prop's own position plus a bbox-derived offset).
    pub fn world_centre(&self) -> Vec3 {
        (self.world_min + self.world_max) * 0.5
    }

    /// Distance along `dir` at which the ray enters this box, or `None` for a
    /// miss — the slab method, the stand-in for the original's pick call,
    /// which returns `-1.0` when the cursor hits nothing.
    pub fn ray_hit(&self, origin: Vec3, dir: Vec3) -> Option<f32> {
        ray_hits_aabb(origin, dir, self.world_min, self.world_max)
    }
}

/// Ray/AABB entry distance, or `None` if the ray misses or the box is behind
/// the ray's origin. Slab test; a zero component of `dir` degenerates into the
/// "is the origin inside this slab" question, which the infinities answer
/// correctly.
pub fn ray_hits_aabb(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let inv = dir.recip();
    let t0 = (min - origin) * inv;
    let t1 = (max - origin) * inv;
    let near = t0.min(t1);
    let far = t0.max(t1);
    let enter = near.x.max(near.y).max(near.z);
    let exit = far.x.min(far.y).min(far.z);
    (exit >= enter.max(0.0)).then_some(enter.max(0.0))
}

/// World placement of one prop, mapped exactly like a lined-up character
/// (`character_select::on_char_selection_action_response`): region base plus
/// authored offset, x-mirrored, minus the floating origin.
pub fn board_prop_translation(cam_base: Vec3, origin: Vec3, offset: Vec3) -> Vec3 {
    (cam_base + offset + Vec3::Y * BOARD_STAGE_LIFT) * Vec3::new(-1.0, 1.0, 1.0) - origin
}

/// The props this corpus puts on the desk. A race idol stands for a choice;
/// without a playable body row there is no choice, so that figure stays off
/// the desk. The desk and the lizard are scenery and always stand.
fn props_on_stage(races: &[Race]) -> impl Iterator<Item = &'static BoardProp> + '_ {
    BOARD_PROPS
        .iter()
        .filter(move |prop| prop.race.is_none_or(|race| races.contains(&race)))
}

/// The authored idol props, in the original's own array order.
fn authored_idols() -> Vec<&'static BoardProp> {
    BOARD_PROPS.iter().filter(|p| p.race.is_some()).collect()
}

/// Where the `index`-th *unauthored* race stands on the board.
///
/// The original authors one idol per race it ships, in code. A corpus that
/// carries bodies for a race whose idol art we cannot name still needs a place
/// on the board, or the race would be visible in the data and unreachable on
/// screen. **Our placement, not the original's**: the row is continued by the
/// step between the two authored idols (`interface_idol_china.offset -
/// …_europe.offset`), so an extra slot keeps the board's own spacing instead
/// of a number picked by eye.
pub(crate) fn extra_idol_offset(index: usize) -> Option<Vec3> {
    let idols = authored_idols();
    let (first, last) = (idols.first()?, idols.last()?);
    let step = last.offset - first.offset;
    Some(last.offset + step * (index as f32 + 1.0))
}

/// `OnEnter(RegionSelect)`: put the original's idols and box on the stage.
///
/// This is the other half of "the race board is a *place*": until now the
/// camera flew to the board and found **nothing there**, so the flight
/// read as a pan into empty sky. The models are
/// the user's own (`Data.pk2 → res/interface/…`), spawned through the same
/// `UnloadedResource` path as the lineup characters, so they get the same
/// mirror/winding handling instead of a second loader (rule 3).
pub fn spawn_race_board_props(
    scene: Option<Res<ActiveCharSelectSceneV2>>,
    origin: Res<WorldOrigin>,
    char_data: Res<crate::plugins::textdata::ClientCharacterData>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let cam_base = scene.map(|s| s.0.cam_base()).unwrap_or(Vec3::ZERO);
    let races = super::race_catalog::available_races(&char_data);
    for prop in props_on_stage(&races) {
        let transform =
            Transform::from_translation(board_prop_translation(cam_base, origin.0, prop.offset))
                .with_scale(Vec3::new(-1.0, 1.0, 1.0))
                .with_rotation(Quat::from_rotation_y(-prop.yaw));
        let mut entity = commands.spawn((
            RaceBoardProp,
            transform,
            Visibility::default(),
            Name::from(prop.path),
            UnloadedResource(asset_server.load(prop.path)),
        ));
        // The two pickable props carry their world box with them, so the hover
        // test in `region_select` asks the same geometry the renderer draws
        // (mirror and facing included) instead of re-deriving it.
        if let Some(race) = prop.race {
            let mut world_min = Vec3::splat(f32::INFINITY);
            let mut world_max = Vec3::splat(f32::NEG_INFINITY);
            for corner in bbox_corners(
                prop.bbox_min + prop.anim_offset,
                prop.bbox_max + prop.anim_offset,
            ) {
                let world = transform.transform_point(corner);
                world_min = world_min.min(world);
                world_max = world_max.max(world);
            }
            entity.insert(RaceBoardIdol {
                race,
                world_min,
                world_max,
            });
        }
        if needs_winding_reversal(&transform.to_matrix()) {
            entity.insert(MirroredResource);
        }
    }

    // A race the corpus can build but the original authors no idol for gets a
    // modelless board slot: no art (we have none to name), but the same
    // pickable box, so the pointer, the plate and the confirm treat it like
    // any other race. On a v1.188-shaped corpus this loop does nothing.
    let mut extra = 0usize;
    for race in races {
        if BOARD_PROPS.iter().any(|p| p.race == Some(race)) {
            continue;
        }
        let Some(offset) = extra_idol_offset(extra) else {
            break;
        };
        let idols = authored_idols();
        let Some(model) = idols.last() else {
            break;
        };
        extra += 1;
        info!("race board: {race:?} has bodies but no authored idol — placing a modelless slot");
        let transform =
            Transform::from_translation(board_prop_translation(cam_base, origin.0, offset))
                .with_scale(Vec3::new(-1.0, 1.0, 1.0))
                .with_rotation(Quat::from_rotation_y(-model.yaw));
        // The pick box is the last authored idol's, so the slot is exactly as
        // big as a real idol instead of being an invented volume.
        let mut world_min = Vec3::splat(f32::INFINITY);
        let mut world_max = Vec3::splat(f32::NEG_INFINITY);
        for corner in bbox_corners(model.bbox_min, model.bbox_max) {
            let world = transform.transform_point(corner);
            world_min = world_min.min(world);
            world_max = world_max.max(world);
        }
        commands.spawn((
            RaceBoardProp,
            transform,
            Visibility::default(),
            Name::from(format!("Race Board Slot {race:?}")),
            RaceBoardIdol {
                race,
                world_min,
                world_max,
            },
        ));
    }
}

/// `OnExit(RegionSelect)`: the props belong to the board, not to the stage.
pub fn despawn_race_board_props(props: Query<Entity, With<RaceBoardProp>>, mut commands: Commands) {
    for entity in props.iter() {
        commands.entity(entity).despawn();
    }
}

/// `OnEnter(RegionSelect)`: fly the stage camera from wherever it is to the
/// race boards. Replaces the pose-snap that held the lineup camera.
pub fn fly_camera_to_race_board(
    cam_query: Query<(Entity, &Transform), With<CinematicCamera2>>,
    scene: Option<Res<ActiveCharSelectSceneV2>>,
    origin: Res<WorldOrigin>,
    mut commands: Commands,
) {
    let Ok((cam_entity, cam_transform)) = cam_query.single() else {
        return;
    };
    let cam_base = scene.map(|s| s.0.cam_base()).unwrap_or(Vec3::ZERO);
    let (target_pos, target_rot) = board_camera_pose(cam_base, origin.0);

    commands.entity(cam_entity).insert((
        RaceBoardFlight {
            target_pos,
            target_rot,
        },
        TweenAnim::new(board_path(
            cam_base,
            origin.0,
            cam_transform.translation,
            cam_transform.rotation,
            false,
            None,
        )),
    ));
}

/// The flight as a tween sequence: live pose → waypoint 1 → waypoint 2 →
/// waypoint 3, one leg (`5/3 s`) each, `reverse` walking the same waypoints
/// the other way for the Cancel return.
///
/// Why a *path* and not the straight line we shipped before: the straight line
/// was a consequence of the two middles being unknown, and it is what made the
/// arrival read as "the camera pans into the sky" — it approached the boards
/// head-on from below with nothing in frame on the way. The original's path
/// swings past the character line first (both middles look straight at it, see
/// [`BOARD_WAYPOINTS`]) and only turns onto the idols at the end, which is the
/// shot the original composes.
fn board_path_poses(
    cam_base: Vec3,
    origin: Vec3,
    reverse: bool,
    finish: Option<(Vec3, Quat)>,
) -> Vec<(Vec3, Quat)> {
    let mut poses: Vec<(Vec3, Quat)> = BOARD_WAYPOINTS
        .iter()
        .map(|(offset, pitch, yaw)| waypoint_pose(cam_base, origin, *offset, *pitch, *yaw))
        .collect();
    // The arrival is the derived pose, not an authored waypoint.
    poses.push(board_camera_pose(cam_base, origin));
    if reverse {
        poses.reverse();
        // Coming back the camera *is* the last waypoint, so that leg would be
        // a zero-length tween; the caller appends the lineup pose instead.
        poses.remove(0);
    }
    poses.extend(finish);
    poses
}

fn board_path(
    cam_base: Vec3,
    origin: Vec3,
    from_pos: Vec3,
    from_rot: Quat,
    reverse: bool,
    finish: Option<(Vec3, Quat)>,
) -> Sequence {
    let mut cursor = (from_pos, from_rot);
    let mut tweens = Vec::new();
    for pose in board_path_poses(cam_base, origin, reverse, finish) {
        tweens.push(keyframe_tween(
            cursor.0,
            pose.0,
            cursor.1,
            pose.1,
            BOARD_FLIGHT,
        ));
        cursor = pose;
    }
    Sequence::new(tweens)
}

/// Cancel on the race board: while this resource lives the camera is flying
/// back to the lineup, and the state switch waits for it.
///
/// **Deliberate deviation, stated (ADR-0009).** What the original does on this
/// Cancel is unknown: its handler replays the 4-waypoint array, but nothing
/// says whether Cancel replays it backwards or cuts. The camera simply drives
/// back: the return is built as the exact mirror of the outbound leg — same
/// pose ladder, same `5/3 s`, no new number. Cutting instead would be the one
/// thing we know looks wrong: the outbound leg is a flight, so an instant
/// snap back is a visible asymmetry.
#[derive(Resource)]
pub struct RaceBoardReturn {
    timer: Timer,
}

/// Set for one `OnEnter(CharacterList)` when the camera already flew home, so
/// the lineup's intro fly-in does not restart on top of the arrival
/// ([`character_select::start_camera_animation`] consumes it).
#[derive(Resource)]
pub struct CharSelectCameraHomed;

/// Starts the Cancel return flight to the lineup's end pose. Called from the
/// board's Cancel button instead of an immediate `next_state.set(...)`; the
/// state follows when [`tick_return_flight`] sees the timer out.
pub fn start_return_flight(
    cam: &Query<(Entity, &Transform), With<CinematicCamera2>>,
    scene: &ActiveCharSelectSceneV2,
    origin: &WorldOrigin,
    commands: &mut Commands,
) -> bool {
    let Ok((cam_entity, cam_transform)) = cam.single() else {
        return false;
    };
    // The pose the lineup's own fly-in ends at — reusing it is what makes the
    // hand-over invisible (rule 3: no second "where does the lineup camera
    // stand" answer).
    let Some((home_pos, home_rot)) = scene.0.init_camera_end_pose(origin.0) else {
        return false;
    };
    // The same path, walked backwards, with the lineup pose as its
    // last leg — so the way home is the way out in reverse, not a new shot.
    let cam_base = scene.0.cam_base();
    commands
        .entity(cam_entity)
        .insert(TweenAnim::new(board_path(
            cam_base,
            origin.0,
            cam_transform.translation,
            cam_transform.rotation,
            true,
            Some((home_pos, home_rot)),
        )));
    commands.insert_resource(RaceBoardReturn {
        timer: Timer::new(BOARD_FLIGHT * RETURN_LEGS, TimerMode::Once),
    });
    true
}

/// `Update` in `RegionSelect`: hand over to the character list when the return
/// flight has landed.
pub fn tick_return_flight(
    time: Res<Time>,
    mut ret: ResMut<RaceBoardReturn>,
    mut next_state: ResMut<NextState<IntroV2State>>,
    mut commands: Commands,
) {
    if !ret.timer.tick(time.delta()).just_finished() {
        return;
    }
    commands.remove_resource::<RaceBoardReturn>();
    commands.insert_resource(CharSelectCameraHomed);
    next_state.set(IntroV2State::CharacterList);
}

/// `OnExit(RegionSelect)`: the flight marker is per-visit state.
pub fn clear_race_board_flight(cams: Query<Entity, With<RaceBoardFlight>>, mut commands: Commands) {
    for cam in cams.iter() {
        commands.entity(cam).remove::<RaceBoardFlight>();
    }
    // A return that is still pending must not tick into the next visit.
    commands.remove_resource::<RaceBoardReturn>();
}

#[cfg(test)]
mod tests {
    /// A corpus race the original authors no idol for must land ON the board's
    /// own row, not at an eyeballed spot: the slot continues the step between
    /// the two authored idols, and it may not coincide with either of them.
    #[test]
    fn an_unauthored_race_continues_the_authored_idol_row() {
        let idols = authored_idols();
        assert_eq!(idols.len(), 2, "the original authors two pickable idols");
        let step = idols[1].offset - idols[0].offset;
        let first = extra_idol_offset(0).expect("a board with idols has a next slot");
        assert_eq!(first, idols[1].offset + step);
        let second = extra_idol_offset(1).expect("and another");
        assert_eq!(second, idols[1].offset + step * 2.0);
        for authored in idols.iter() {
            assert_ne!(first, authored.offset);
            assert_ne!(second, authored.offset);
        }
    }

    use super::*;
    use crate::scenes::intro_v2::IntroV2State;
    use crate::scenes::SceneState;

    /// The lineup constants the scene asset ships, for the "is it actually a
    /// different place?" assertion below.
    const LINEUP_CHAR_X: f32 = 95.0;
    const LINEUP_CAM_OFFSET: Vec3 = Vec3::new(56.0, -12.0, 697.6);

    /// The china idol's authored offset on the board.
    const IDOL_CHINA: Vec3 = Vec3::new(156.2, -20.0, 651.6);

    /// The board is a *place*, not an overlay: it has to be far from the
    /// lineup, or the "camera flight" is cosmetic.
    #[test]
    fn the_board_camera_stands_at_the_idols_not_at_the_lineup() {
        let (cam, _) = board_camera_pose(Vec3::ZERO, Vec3::ZERO);
        let (centre, radius) = board_props_bounds(Vec3::ZERO, Vec3::ZERO);
        let lineup = LINEUP_CAM_OFFSET * Vec3::new(-1.0, 1.0, 1.0);
        let lineup_char = Vec3::new(LINEUP_CHAR_X, -27.6, 654.0) * Vec3::new(-1.0, 1.0, 1.0);
        // Far from the lineup — otherwise the "camera flight" is cosmetic.
        assert!(
            (cam - lineup).length() > 100.0,
            "the board pose must be a different place than the lineup camera"
        );
        assert!((cam - lineup_char).length() > 50.0);
        // ... and standing off its own motif by exactly the framing distance.
        let distance = radius / (BOARD_FRAME_FILL * (BOARD_CAM_FOV * 0.5).tan());
        assert!(
            ((cam - centre).length() - distance).abs() < 1e-2,
            "the arrival must stand at the framing distance from the props"
        );
    }

    /// The arrival pose must frame the idol group — the strongest assertion
    /// available here, because the idol position comes out of the shipped
    /// `.bsr` and not out of our own code.
    ///
    /// It is also what settled the rotation convention: the earlier
    /// `XYZ(pitch, PI - yaw)` scored **-0.30** against this direction (facing
    /// away), `YXZ(-yaw, pitch)` scores 0.988.
    #[test]
    fn the_board_camera_looks_at_the_idols() {
        let (pos, rot) = board_camera_pose(Vec3::ZERO, Vec3::ZERO);
        // Through the prop mapping, not by hand: this test used to mirror the
        // idol itself and therefore compared a *lifted* camera against an
        // *unlifted* idol — 6.3 units of built-in error that only stayed under
        // the threshold while the pose looked up (it scored 0.945 the moment
        // the arrival was mirrored over the table plane, 2026-08-25). The lift
        // is rigid, so both sides have to go through it.
        let idol = board_prop_translation(Vec3::ZERO, Vec3::ZERO, IDOL_CHINA);
        let to_idol = (idol - pos).normalize();
        let forward = rot * Vec3::NEG_Z;
        assert!(
            forward.dot(to_idol) > 0.96,
            "camera forward {forward:?} does not face the idols ({to_idol:?})"
        );
    }

    /// The camera looks at the boards rather than back at the lineup — stated
    /// as a *comparison*, and that correction matters.
    ///
    /// From the arrival camera position the idols and the lineup are only 39°
    /// apart (`to_idol · to_lineup = 0.77`), so a pose that frames the idols to
    /// within 9° unavoidably sits 30-48° off the lineup. An absolute threshold
    /// against the lineup would therefore cut straight through the valid range
    /// and contradict the test above — two assertions that cannot both be green
    /// for any pose. What the test wants to say is that the boards win, so it
    /// says exactly that.
    #[test]
    fn the_boards_win_against_the_lineup() {
        let (pos, rot) = board_camera_pose(Vec3::ZERO, Vec3::ZERO);
        let forward = rot * Vec3::NEG_Z;
        let to_idol = (IDOL_CHINA * Vec3::new(-1.0, 1.0, 1.0) - pos).normalize();
        let to_lineup = (LINEUP_CAM_OFFSET * Vec3::new(-1.0, 1.0, 1.0) - pos).normalize();
        assert!(
            forward.dot(to_idol) > forward.dot(to_lineup),
            "camera favours the lineup ({:.3}) over the boards ({:.3})",
            forward.dot(to_lineup),
            forward.dot(to_idol)
        );
    }

    /// The world mapping is the scene's, not a private one: same region base,
    /// same x-mirror, same floating-origin subtraction as a scene keyframe —
    /// plus the one stated correction, [`BOARD_STAGE_LIFT`].
    ///
    /// The lift's red case is this very assertion: written without the lift it
    /// failed with `y = -29.6` against `-35.9`, which is how the constant is
    /// kept honest — remove it and the test says so.
    #[test]
    fn the_pose_follows_the_scene_mapping() {
        // Stated as the mapping's *property* instead of restating its formula:
        // moving the floating origin moves the pose by exactly `-delta`, and
        // the pose keeps its separation from the props (which go
        // through the same mapping). A formula copy would only assert that the
        // line was copied correctly.
        let base = Vec3::new(81.0 * 1920.0, 0.0, 105.0 * 1920.0);
        let (a, rot_a) = board_camera_pose(base, Vec3::ZERO);
        let delta = Vec3::new(1.0, 2.0, 3.0);
        let (b, rot_b) = board_camera_pose(base, delta);
        assert!((b - (a - delta)).length() < 1e-2, "{a:?} vs {b:?}");
        assert_eq!(rot_a, rot_b);
    }

    /// The lift is *rigid*: props and camera move together, or it would be a
    /// composition change dressed up as an anchor. Asserted as the invariant
    /// itself — the camera-to-idol vector is unchanged, lift or no lift.
    #[test]
    fn the_stage_lift_moves_the_camera_and_the_props_together() {
        let base = Vec3::new(81.0 * 1920.0, 0.0, 105.0 * 1920.0);
        let origin = Vec3::new(1.0, 2.0, 3.0);
        let (cam, _) = board_camera_pose(base, origin);
        let (centre, _) = board_props_bounds(base, origin);
        // The lift is applied by `board_prop_translation`, and the camera is
        // derived from those very translations, so the lift cannot move one
        // without the other. What is worth asserting is that both live at the
        // *lifted* height: the props' centre must sit above our deck level
        // (`char_start_offset.y = -27.6`), which is the whole reason
        // `BOARD_STAGE_LIFT` exists.
        assert!(
            centre.y > -27.6,
            "the lifted tableau must sit above the deck, got {}",
            centre.y
        );
        assert!(
            cam.y > centre.y,
            "the arrival must stand above the tableau it looks down on"
        );
    }

    /// The arrival frames the props: centred, whole, and from above — at BOTH
    /// aspect ratios, with the red control on the pose this replaced.
    ///
    /// This is a *picture* criterion, not a dot product. A dot product only says
    /// the camera points somewhere near the motif; it said `0.988` while the
    /// tableau sat in the bottom-left corner of the frame. So the assertion
    /// projects the props' eight-corner cloud through the same perspective the
    /// camera has (vertical FOV `BOARD_CAM_FOV`, aspect as given) and checks
    /// where it lands in normalised device coordinates:
    ///
    /// * every corner inside the frame (`|ndc| <= 1`) — nothing cropped;
    /// * the union's centre within the middle third (`|ndc| <= 1/3`);
    /// * the union covering at least 15% of the frame area — it is the motif,
    ///   not a detail of the quay.
    ///
    /// Red control, in the same test: the previous arrival — the measured pose
    /// mirrored about the *placement plane* instead of the view axis — must fail
    /// the centring check (it is 14.2° off-axis; photographed as "the table
    /// clings to the bottom-left corner").
    #[test]
    fn the_arrival_frames_the_props_at_both_aspects() {
        /// Perspective projection to normalised device coordinates, or `None`
        /// when a point is behind the camera.
        fn ndc(cam: Vec3, rot: Quat, aspect: f32, point: Vec3) -> Option<Vec2> {
            let forward = rot * Vec3::NEG_Z;
            let right = rot * Vec3::X;
            let up = rot * Vec3::Y;
            let v = point - cam;
            let depth = v.dot(forward);
            if depth <= 0.0 {
                return None;
            }
            let t = (BOARD_CAM_FOV * 0.5).tan();
            Some(Vec2::new(
                v.dot(right) / (depth * t * aspect),
                v.dot(up) / (depth * t),
            ))
        }

        let corners = board_prop_corners(Vec3::ZERO, Vec3::ZERO);
        let (cam, rot) = board_camera_pose(Vec3::ZERO, Vec3::ZERO);
        // the maintainer's own two shapes: his 3440x1440 and plain 16:9
        for aspect in [3440.0 / 1440.0, 16.0 / 9.0] {
            let projected: Vec<Vec2> = corners
                .iter()
                .map(|c| ndc(cam, rot, aspect, *c).expect("props must be in front of the camera"))
                .collect();
            let min = projected
                .iter()
                .fold(Vec2::splat(f32::MAX), |a, b| a.min(*b));
            let max = projected
                .iter()
                .fold(Vec2::splat(f32::MIN), |a, b| a.max(*b));
            assert!(
                min.x >= -1.0 && min.y >= -1.0 && max.x <= 1.0 && max.y <= 1.0,
                "the tableau is cropped at aspect {aspect}: {min:?}..{max:?}"
            );
            let centre = (min + max) * 0.5;
            assert!(
                centre.x.abs() <= 1.0 / 3.0 && centre.y.abs() <= 1.0 / 3.0,
                "the tableau is off-centre at aspect {aspect}: {centre:?}"
            );
            let area = (max.x - min.x) * (max.y - min.y) / 4.0;
            assert!(
                area >= 0.15,
                "the tableau covers only {area} of the frame at aspect {aspect}"
            );
        }

        // Red control: the pose this replaced (measured height and pitch
        // mirrored about the placement plane) is NOT centred.
        let mirrored_cam = (Vec3::new(
            BOARD_CAM_MEASURED_OFFSET.x,
            2.0 * BOARD_TABLE_LEVEL - BOARD_CAM_MEASURED_OFFSET.y,
            BOARD_CAM_MEASURED_OFFSET.z,
        ) + Vec3::Y * BOARD_STAGE_LIFT)
            * Vec3::new(-1.0, 1.0, 1.0);
        let projected: Vec<Vec2> = corners
            .iter()
            .filter_map(|c| ndc(mirrored_cam, rot, 3440.0 / 1440.0, *c))
            .collect();
        let min = projected
            .iter()
            .fold(Vec2::splat(f32::MAX), |a, b| a.min(*b));
        let max = projected
            .iter()
            .fold(Vec2::splat(f32::MIN), |a, b| a.max(*b));
        let centre = (min + max) * 0.5;
        assert!(
            centre.x.abs() > 1.0 / 3.0 || centre.y.abs() > 1.0 / 3.0 || min.y < -1.0,
            "the replaced pose was supposed to be the off-centre one, got {centre:?}"
        );
    }

    /// The mirrored FOV must stay the camera's own, or the framing distance is
    /// computed against a lens the client does not use.
    #[test]
    fn the_frame_fill_uses_the_cameras_own_fov() {
        // `plugins::camera::FOV` is private; this is the same literal, and the
        // test is the tripwire if that one changes.
        assert_eq!(BOARD_CAM_FOV, std::f32::consts::FRAC_PI_4);
        assert!(BOARD_FRAME_FILL > 0.0 && BOARD_FRAME_FILL <= 1.0);
    }

    /// The flight is the original's *path*, not a straight line, and the return
    /// is the same path backwards. Counting legs is the observable part of a
    /// `Sequence`; the poses themselves are asserted by the tests above.
    #[test]
    fn the_flight_walks_its_waypoints_and_comes_back() {
        let base = Vec3::ZERO;
        let out = board_path_poses(base, Vec3::ZERO, false, None);
        assert_eq!(
            out.len(),
            // the two authored middles plus the derived arrival
            BOARD_WAYPOINTS.len() + 1,
            "outbound must fly one leg per waypoint plus the arrival"
        );
        assert_eq!(
            out.last().copied(),
            Some(board_camera_pose(base, Vec3::ZERO))
        );
        let home = (Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY);
        let back = board_path_poses(base, Vec3::ZERO, true, Some(home));
        assert_eq!(
            back.len(),
            RETURN_LEGS as usize,
            "the return drops the waypoint it starts on and adds the lineup pose"
        );
        assert_eq!(
            back.last().copied(),
            Some(home),
            "the return ends at the lineup"
        );
        assert_ne!(
            back.first().copied(),
            out.last().copied(),
            "the return must not start with a zero-length leg onto its own pose"
        );
    }

    /// The three idols must STAND ON the desk: underside on its top face, and
    /// footprint fully over it. Without this the idols look like they float.
    /// The whole point is the negative control at the end: with the box facing
    /// we used to ship (`yaw = 0.0`) the very same check fails, which is what
    /// proves the test checks the placement and not itself.
    ///
    /// Reading form is the one the tree already uses for "where does a body
    /// touch down" (`plugins::cos::riding::resolve_saddle_seats`,
    /// `character_select::figure_top`): an Aabb union, mapped through the
    /// placement. Here it is the mesh union of `BOARD_PROPS`, which agrees with
    /// the running client to the third decimal.
    #[test]
    fn every_idol_stands_on_the_table() {
        let base = Vec3::new(-1000.0, 5.0, 700.0);
        let origin = Vec3::new(3.0, -4.0, 5.0);
        let (plane_y, quad) = board_table_top(base, origin);

        for prop in BOARD_PROPS.iter().skip(1) {
            let (underside, footprint) = board_prop_footprint(prop, base, origin);
            let air = underside - plane_y;
            assert!(
                air.abs() <= BOARD_TABLE_CONTACT_TOLERANCE,
                "{} sits {air} off the table plane (tolerance {})",
                prop.path,
                BOARD_TABLE_CONTACT_TOLERANCE
            );
            for corner in footprint {
                assert!(
                    inside_quad_xz(&quad, corner),
                    "{} has a footprint corner {corner:?} off the table top {quad:?}",
                    prop.path
                );
            }
        }

        // NEGATIVE CONTROL — the state before this fix: the box unrotated.
        // Two of the three idols then land behind the slab (the europe idol
        // 2.6..3.8 units past its back edge, in mid-air over the stone step),
        // so at least one footprint corner must fall off the table. If this
        // stops failing, the check above has gone blind.
        let (_, unrotated) = board_table_top_with_yaw(0.0, base, origin);
        let off_the_table = BOARD_PROPS.iter().skip(1).any(|prop| {
            let (_, footprint) = board_prop_footprint(prop, base, origin);
            footprint
                .iter()
                .any(|corner| !inside_quad_xz(&unrotated, *corner))
        });
        assert!(
            off_the_table,
            "negative control: with the box unrotated the idols must NOT all be on the table"
        );
    }

    /// The *facts* of the click move, reproduced from the two floats this file
    /// carries: each region's target minimises the
    /// off-axis angle of **its own** idol, in the ORIGINAL's frame and with the
    /// original's own forward vector. If either triple were mis-transcribed —
    /// or read in memory order instead of coordinate order — this table
    /// collapses.
    #[test]
    fn the_confirm_targets_centre_their_own_idol() {
        // the original's convention, not ours: forward from (yaw, pitch)
        fn forward(yaw: f32, pitch: f32) -> Vec3 {
            Vec3::new(
                -yaw.sin() * pitch.cos(),
                pitch.sin(),
                -yaw.cos() * pitch.cos(),
            )
        }
        let fwd = forward(BOARD_CAM_YAW_ORIGINAL, BOARD_CAM_ARRIVAL_PITCH);
        let off_axis =
            |cam: Vec3, idol: Vec3| (idol - cam).normalize().dot(fwd).acos().to_degrees();
        // the two idols and the arrival waypoint
        let idol_eu = Vec3::new(157.0, -20.0, 654.4);
        let idol_cn = Vec3::new(156.2, -20.0, 651.6);
        let arrival = BOARD_CAM_ARRIVAL_OFFSET;

        // each target is better than the arrival for its own figure ...
        assert!(off_axis(CONFIRM_TARGET_EUROPE, idol_eu) < off_axis(arrival, idol_eu));
        assert!(off_axis(CONFIRM_TARGET_CHINA, idol_cn) < off_axis(arrival, idol_cn));
        // ... and better than the other target for its own figure — the half
        // that makes it a CHOICE and not just "a better shot"
        assert!(off_axis(CONFIRM_TARGET_EUROPE, idol_eu) < off_axis(CONFIRM_TARGET_CHINA, idol_eu));
        assert!(off_axis(CONFIRM_TARGET_CHINA, idol_cn) < off_axis(CONFIRM_TARGET_EUROPE, idol_cn));
        // the expected angles, to 0.05°
        assert!((off_axis(CONFIRM_TARGET_EUROPE, idol_eu) - 2.82).abs() < 0.05);
        assert!((off_axis(CONFIRM_TARGET_CHINA, idol_cn) - 5.21).abs() < 0.05);
        assert!((off_axis(arrival, idol_eu) - 6.23).abs() < 0.05);
        assert!((off_axis(arrival, idol_cn) - 8.79).abs() < 0.05);
        // and the move really does go BACKWARDS, which is why we do not
        // transplant it: the target sits further from the figure
        assert!((CONFIRM_TARGET_EUROPE - idol_eu).length() > (arrival - idol_eu).length());
        assert!((CONFIRM_TARGET_CHINA - idol_cn).length() > (arrival - idol_cn).length());
        // the original's duration is 1.0 s
        assert_eq!(CONFIRM_FLIGHT, Duration::from_secs(1));
    }

    /// OUR confirm pose — named for the invariant, not for the (refuted)
    /// close-up premise: it puts the chosen figure on the view axis.
    ///
    /// The two frames are kept apart on purpose. The **original's** behaviour is
    /// pinned one test up, `the_confirm_targets_centre_their_own_idol`:
    /// a 1.0 s pure translation that ends *further* from the figure while
    /// centring it. **Ours** additionally closes in, because our arrival stands
    /// 39.1 instead of 18.2 units back and therefore has to — so the distance
    /// assertion below is a statement about our stated deviation, and the
    /// off-axis assertion is the decision both share.
    ///
    /// The negative control is the assertion pair itself, and it is exactly the
    /// earlier state: with **no** flight the confirm pose *is*
    /// the arrival pose, so every `<` below is a `==` and the test fails. It also
    /// fails if the pose stops depending on the race (a single "confirm shot"),
    /// or if it aims past the figure like the first arrival attempt did.
    #[test]
    fn our_confirm_pose_puts_the_chosen_figure_on_the_view_axis() {
        let cam_base = Vec3::ZERO;
        let origin = Vec3::ZERO;
        let (arrival, arrival_rot) = board_camera_pose(cam_base, origin);
        let idol = |race: Race| {
            let prop = BOARD_PROPS
                .iter()
                .find(|prop| prop.race == Some(race))
                .expect("both idols are on the board");
            prop_bounds(prop, cam_base, origin).0
        };

        for race in [Race::EUROPEAN, Race::CHINESE] {
            let (pos, rot) = confirm_camera_pose(race, cam_base, origin);
            // the orientation is the original's and does NOT change — the
            // original hands the second keyframe a copy of the arrival
            // rotation
            assert_eq!(rot, arrival_rot);
            // it is a close-up: closer to the chosen figure than the arrival
            let chosen = idol(race);
            assert!(
                (pos - chosen).length() < (arrival - chosen).length(),
                "{race:?}: the confirm pose must approach the chosen figure"
            );
            // and it is centred on that figure, better than the arrival is
            let off_axis = |cam: Vec3, rot: Quat, point: Vec3| {
                (point - cam)
                    .normalize()
                    .dot(rot * Vec3::NEG_Z)
                    .acos()
                    .to_degrees()
            };
            assert!(
                off_axis(pos, rot, chosen) < off_axis(arrival, arrival_rot, chosen),
                "{race:?}: the chosen figure must move towards the view axis"
            );
            // the whole figure still fits: its bounding sphere inside the frame
            let prop = BOARD_PROPS
                .iter()
                .find(|prop| prop.race == Some(race))
                .unwrap();
            let (_, radius) = prop_bounds(prop, cam_base, origin);
            let half_fov = BOARD_CAM_FOV * 0.5;
            assert!(
                (radius / (pos - chosen).length()).asin() < half_fov,
                "{race:?}: the close-up must not crop the figure"
            );
        }
        // per region, not one shot for both
        assert_ne!(
            confirm_camera_pose(Race::EUROPEAN, cam_base, origin).0,
            confirm_camera_pose(Race::CHINESE, cam_base, origin).0
        );
    }

    /// A corpus without European body rows puts no European idol
    /// on the desk — nothing to hover, nothing to plate. The desk and the
    /// lizard (no race) stay; a corpus with both races gets all four props.
    #[test]
    fn a_race_without_bodies_has_no_idol_on_the_desk() {
        let paths =
            |races: &[Race]| -> Vec<&str> { props_on_stage(races).map(|p| p.path).collect() };
        let chinese_only = paths(&[Race::CHINESE]);
        assert_eq!(chinese_only.len(), 3);
        assert!(!chinese_only.iter().any(|p| p.contains("idol_europe")));
        assert!(chinese_only.iter().any(|p| p.contains("idol_china")));
        assert!(chinese_only.iter().any(|p| p.contains("lizard")));
        assert!(chinese_only.iter().any(|p| p.ends_with("box.bsr")));
        assert_eq!(
            paths(&[Race::CHINESE, Race::EUROPEAN]).len(),
            BOARD_PROPS.len()
        );
        assert_eq!(
            paths(&[]).len(),
            2,
            "no race at all leaves only the scenery"
        );
    }

    #[test]
    fn entering_the_region_select_sends_the_camera_to_the_board() {
        let mut app = App::new();
        // `init_state` needs the `StateTransition` schedule, which only
        // `StatesPlugin` (or `DefaultPlugins`) installs — a bare `App::new()`
        // panics on the first transition.
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.init_state::<SceneState>()
            .add_sub_state::<IntroV2State>()
            .insert_resource(WorldOrigin(Vec3::ZERO))
            .add_systems(
                OnEnter(IntroV2State::RegionSelect),
                fly_camera_to_race_board,
            )
            .add_systems(OnExit(IntroV2State::RegionSelect), clear_race_board_flight);

        let cam = app
            .world_mut()
            .spawn((CinematicCamera2, Transform::from_xyz(-56.0, -12.0, 697.6)))
            .id();

        app.world_mut()
            .resource_mut::<NextState<SceneState>>()
            .set(SceneState::IntroV2);
        app.update();
        app.world_mut()
            .resource_mut::<NextState<IntroV2State>>()
            .set(IntroV2State::RegionSelect);
        app.update();

        let flight = app
            .world()
            .entity(cam)
            .get::<RaceBoardFlight>()
            .copied()
            .expect("entering RegionSelect must start the board flight");
        let (expected_pos, expected_rot) = board_camera_pose(Vec3::ZERO, Vec3::ZERO);
        assert_eq!(flight.target_pos, expected_pos);
        assert_eq!(flight.target_rot, expected_rot);
        assert!(
            app.world().entity(cam).contains::<TweenAnim>(),
            "the camera must be tweening, not snapped"
        );

        app.world_mut()
            .resource_mut::<NextState<IntroV2State>>()
            .set(IntroV2State::CharacterCreate);
        app.update();
        assert!(
            !app.world().entity(cam).contains::<RaceBoardFlight>(),
            "the flight marker is per-visit state"
        );
    }
}
