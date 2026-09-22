use std::time::Duration;

use crate::assets::intro_scene::CameraKeyframe;
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::math::{EulerRot, Quat};
use bevy::prelude::{Asset, TypePath, Vec3};
use bevy_tweening::Sequence;
use serde_derive::{Deserialize, Serialize};
use thiserror::Error;

use crate::plugins::map::terrain::REGION_SIZE;
use crate::scenes::intro_v2::character_create::Race;
use crate::util::tweening_ext::keyframe_tween;

#[derive(TypePath, Asset, Serialize, Deserialize, Default, Debug, Clone)]
pub struct CharSelectScene {
    name: String,
    cam_base: Vec3,
    cam_offset: Vec3,
    char_start_offset: Vec3,
    char_end_offset: Vec3,
    initial_camera_transforms: Vec<CameraKeyframe>,
    select_camera_transforms: Vec<CameraKeyframe>,
    create_stages: CreateStages,
}

/// Where character *creation* stands, per race.
///
/// The idea: in the original, creation is not a camera move on the selection
/// stage — it is a different **place in the world**. Each creation screen
/// writes a region id and a local position into the stage anchor; the
/// original's two creation screens are the same screen with different
/// constants. So the stage is `(region, local offset)` — exactly
/// the shape a [`CameraKeyframe`] already has (`rx`/`rz` + `offset`), which is
/// why this is data and not code.
///
/// Exactly **two** fields, not a map: the v1.188 client has no third creation
/// screen (a race the table carries but this file does not name stands on the
/// un-suffixed stage, see `create_stage`).
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct CreateStages {
    chinese: CreateStage,
    european: CreateStage,
}

/// One race's creation stage: where the camera stands, and where the body it
/// is customising stands. Both are local to the stage's own region, so the
/// pair survives the world origin moving.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct CreateStage {
    camera: CameraKeyframe,
    character: Vec3,
}

#[derive(Error, Debug)]
pub enum CharSelectSceneLoaderError {
    #[error("failed to parse yaml: {0}")]
    YAML(serde_yaml::Error),
    #[error("IO Error: {0}")]
    IO(std::io::Error),
}

impl AssetLoader for CharSelectScene {
    type Asset = CharSelectScene;
    type Settings = ();
    type Error = CharSelectSceneLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut buf = Vec::new();
        let _ = reader
            .read_to_end(&mut buf)
            .await
            .map_err(CharSelectSceneLoaderError::IO);
        let scene: Self::Asset =
            serde_yaml::from_slice(&buf).map_err(CharSelectSceneLoaderError::YAML)?;
        Ok(scene)
    }

    fn extensions(&self) -> &[&str] {
        &["selection"]
    }
}

impl CameraKeyframe {
    fn rotation_without_y_offset(&self) -> Quat {
        Quat::from_euler(
            EulerRot::XYZ,
            self.rotation.x,
            self.rotation.y,
            self.rotation.z,
        )
    }
}

impl CharSelectScene {
    #[allow(dead_code)]
    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn cam_base(&self) -> Vec3 {
        self.cam_base * Vec3::new(1920.0, 0.0, 1920.0)
    }
    pub fn cam_offset(&self) -> Vec3 {
        self.cam_offset
    }
    pub fn char_start_offset(&self) -> Vec3 {
        self.char_start_offset
    }
    pub fn char_end_offset(&self) -> Vec3 {
        self.char_end_offset
    }

    pub fn get_init_camera_anim(&self, origin: Vec3) -> Sequence {
        let mut tweens = Vec::with_capacity(self.initial_camera_transforms.len() - 1);

        for i in 0..self.initial_camera_transforms.len() - 1 {
            let start = self.initial_camera_transforms[i];
            let end = self.initial_camera_transforms[i + 1];

            let duration = Duration::from_secs_f32(end.frame - start.frame);

            tweens.push(keyframe_tween(
                start.translation(origin),
                end.translation(origin),
                start.rotation_without_y_offset(),
                end.rotation_without_y_offset(),
                duration,
            ));
        }

        Sequence::new(tweens)
    }

    /// Final pose of the initial camera animation: the canonical zoomed-out
    /// view the camera returns to when a selection is cancelled. Using the
    /// last keyframe (instead of the live camera transform) keeps the zoom-out
    /// target deterministic even when a character is clicked mid-animation.
    pub fn init_camera_end_pose(&self, origin: Vec3) -> Option<(Vec3, Quat)> {
        self.initial_camera_transforms
            .last()
            .map(|kf| (kf.translation(origin), kf.rotation_without_y_offset()))
    }

    /// The creation stage of `race`.
    /// The original hard-wires its creation stages in code, one class per
    /// race, and ships exactly two of them (`…CreateIslam` does not exist —
    /// see [`CreateStages`]). We keep that: a race whose stage we cannot cite
    /// stands on the un-suffixed one instead of on an invented region.
    fn create_stage(&self, race: Race) -> &CreateStage {
        if race == Race::EUROPEAN {
            &self.create_stages.european
        } else {
            &self.create_stages.chinese
        }
    }

    /// The world origin the creation stage wants: its region's grid point,
    /// mirrored into our render convention the same way [`Self::cam_base`] is.
    /// Anchoring here is what makes the terrain/object/compound/foliage chain
    /// stream the *creation* region instead of the selection one — nothing in
    /// that chain needs to know a screen changed, it only reads `WorldOrigin`.
    pub fn create_stage_anchor(&self, race: Race) -> Vec3 {
        let camera = &self.create_stage(race).camera;
        Vec3::new(camera.rx * REGION_SIZE, 0.0, camera.rz * REGION_SIZE) * Vec3::new(-1.0, 1.0, 1.0)
    }

    /// Which way the stage camera faces, as a yaw. The body is turned to meet
    /// it, so this is the one number both of them read.
    pub fn create_stage_facing(&self, race: Race) -> f32 {
        self.create_stage(race).camera.rotation.y
    }

    /// Where the body being customised stands on that stage, in render space.
    /// Local to the stage's own region for the same reason the camera is:
    /// the selection stage's `char_*_offset` are local to *its* region, and
    /// reusing them here left the figure half a map behind — an empty Jangan
    /// courtyard.
    pub fn create_character_position(&self, race: Race, origin: Vec3) -> Vec3 {
        let stage = self.create_stage(race);
        let mirror = Vec3::new(-1.0, 1.0, 1.0);
        let base = Vec3::new(
            stage.camera.rx * REGION_SIZE,
            0.0,
            stage.camera.rz * REGION_SIZE,
        ) * mirror;
        (base - origin) + stage.character * mirror
    }

    /// Held camera pose of the character-creation stage. One authored key per
    /// race, so this is a pose-set and never a tween: the original does not
    /// animate here either — slot 10 writes the anchor in one go and the
    /// loading screen covers the region swap.
    pub fn create_camera_pose(&self, race: Race, origin: Vec3) -> (Vec3, Quat) {
        let camera = &self.create_stage(race).camera;
        (
            camera.translation(origin),
            camera.rotation_without_y_offset(),
        )
    }

    #[allow(dead_code)]
    pub fn get_select_camera_anim(&self, origin: Vec3) -> Sequence {
        let mut tweens = Vec::with_capacity(self.select_camera_transforms.len() - 1);

        for i in 0..self.select_camera_transforms.len() - 1 {
            let start = self.select_camera_transforms[i];
            let end = self.select_camera_transforms[i + 1];

            let duration = Duration::from_secs_f32(end.frame - start.frame);

            tweens.push(keyframe_tween(
                start.translation(origin),
                end.translation(origin),
                start.rotation_without_y_offset(),
                end.rotation_without_y_offset(),
                duration,
            ));
        }

        Sequence::new(tweens)
    }
}

#[cfg(test)]
mod tests {
    use crate::assets::char_select_scene::CharSelectScene;
    use crate::plugins::map::terrain::REGION_SIZE;
    use crate::scenes::intro_v2::character_create::Race;
    use bevy::prelude::Vec3;

    /// The shipped stage data, so these tests guard the asset and not a copy.
    fn shipped() -> CharSelectScene {
        serde_yaml::from_str(include_str!(
            "../../../assets/char_selects/constantinople.selection"
        ))
        .expect("constantinople.selection must parse")
    }

    /// The two creation screens stand in two *different* world regions —
    /// Jangan `0x62A8` = (168, 98) and Constantinople `0x694F` = (79, 105),
    /// as each creation screen states them. Before this was data, both pointed
    /// at the selection stage's `81/105`.
    #[test]
    fn each_race_creates_in_its_own_region() {
        let scene = shipped();
        let china = scene.create_stage_anchor(Race::CHINESE);
        let europe = scene.create_stage_anchor(Race::EUROPEAN);

        assert_eq!(
            china,
            Vec3::new(-168.0 * REGION_SIZE, 0.0, 98.0 * REGION_SIZE)
        );
        assert_eq!(
            europe,
            Vec3::new(-79.0 * REGION_SIZE, 0.0, 105.0 * REGION_SIZE)
        );
        assert_ne!(china, europe, "the two stages are half a map apart");
        // and neither of them is the selection stage
        assert_ne!(china, scene.cam_base() * Vec3::new(-1.0, 1.0, 1.0));
    }

    /// Anchored on its own region, the creation camera stands over exactly the
    /// ground point the binary writes — mirrored in x, which is the only thing
    /// our convention does to SRO coordinates. The height is deliberately
    /// ours (see the file's own comment), so only x/z are asserted; asserting a
    /// rounded 960.4 would pass on a camera a whole f32 grid step off, which is
    /// the bug this pair of tests found.
    #[test]
    fn create_camera_stands_over_the_ground_point() {
        let scene = shipped();
        for (race, x, z) in [
            // x / z as the original's creation screens write them
            (Race::CHINESE, 960.4188843_f32, 458.2597656_f32),
            (Race::EUROPEAN, 335.0_f32, 1460.0_f32),
        ] {
            let origin = scene.create_stage_anchor(race);
            let body = scene.create_character_position(race, origin);
            assert_eq!(body.x, -x, "{race:?} body x");
            assert_eq!(body.z, z, "{race:?} body z");

            // and its camera is on the same stage looking at it, not on the
            // selection stage half a map away.
            let (camera, _) = scene.create_camera_pose(race, origin);
            let distance = body.distance(camera);
            assert!(
                (distance - 51.0).abs() < 1.0,
                "{race:?} camera is {distance} from its subject, not the \
                 selection stage's 48.6 + 15.6"
            );
        }
    }

    #[test]
    fn rejects_empty_scene() {
        let yaml = r#"
        "#;

        let parsed = serde_yaml::from_str::<CharSelectScene>(&yaml);
        assert!(parsed.is_err());
    }
}
