//! Image marks that load by URL. Each draws a gray placeholder until its image arrives, and the
//! window draws each image as it arrives. The last URL fails, so its placeholder stays.

use std::sync::Arc;
use std::time::Duration;

use avenger_app::app::{AppBuild, AvengerApp, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_common::types::{ImageAlign, TextAlign, TextBaseline};
use avenger_image::reqwest_fetcher::ReqwestImageFetcher;
use avenger_image::{ImageCache, ImageCacheOptions, RgbaImage};
use avenger_scenegraph::marks::image::{SceneImageMark, SceneImageResource};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

/// Vega's sample images, and a URL that doesn't exist.
const IMAGES: [&str; 4] = [
    "https://vega.github.io/vega-datasets/data/ffox.png",
    "https://vega.github.io/vega-datasets/data/gimp.png",
    "https://vega.github.io/vega-datasets/data/7zip.png",
    "https://vega.github.io/vega-datasets/data/missing.png",
];

/// The side of each image's square, and the gap between them.
const SIDE: f32 = 128.0;
const GAP: f32 = 32.0;

struct State;

/// The images' scene.
const IMAGES_SCENE: &str = "images";

struct Images;

#[async_trait::async_trait]
impl AppBuild<State> for Images {
    fn scenes(&self) -> Vec<Scene<State>> {
        vec![Scene::new(IMAGES_SCENE, 0)]
    }

    async fn build(
        &self,
        _state: &mut State,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if !scenes.requested(IMAGES_SCENE) {
            return Ok(());
        }
        let placeholder = Arc::new(RgbaImage {
            width: 1,
            height: 1,
            data: vec![210, 214, 220, 255],
        });
        let x: Vec<f32> = (0..IMAGES.len())
            .map(|index| GAP + index as f32 * (SIDE + GAP) + SIDE / 2.0)
            .collect();
        let images = SceneImageMark {
            len: IMAGES.len() as u32,
            image: IMAGES
                .iter()
                .map(|url| {
                    SceneImageResource {
                        url: url.to_string(),
                        width: SIDE as u32,
                        height: SIDE as u32,
                        placeholder: Some(placeholder.clone()),
                    }
                    .into()
                })
                .collect::<Vec<_>>()
                .into(),
            x: x.clone().into(),
            y: GAP.into(),
            width: SIDE.into(),
            height: SIDE.into(),
            align: ImageAlign::Center.into(),
            ..Default::default()
        };
        let names = SceneTextMark {
            len: IMAGES.len() as u32,
            text: IMAGES
                .iter()
                .map(|url| url.rsplit('/').next().unwrap_or(url).to_string())
                .collect::<Vec<_>>()
                .into(),
            x: x.into(),
            y: (GAP + SIDE + 22.0).into(),
            align: TextAlign::Center.into(),
            baseline: TextBaseline::Alphabetic.into(),
            font: "Lato".to_string().into(),
            font_size: 13.0.into(),
            color: [0.09, 0.17, 0.26, 1.0].into(),
            ..Default::default()
        };
        let marks: Vec<SceneMark> = vec![images.into(), names.into()];
        let [width, height] = size();
        let scene = SceneGraph {
            width,
            height,
            origin: [0.0, 0.0],
            marks,
        };
        scenes.set(IMAGES_SCENE, scene.into())
    }
}

/// Return the window size for a row of images with labels below.
fn size() -> [f32; 2] {
    [
        GAP + IMAGES.len() as f32 * (SIDE + GAP),
        SIDE + 2.0 * GAP + 30.0,
    ]
}

async fn run() {
    env_logger::init();
    let fetcher = ReqwestImageFetcher::new(
        concat!(
            "avenger-image-resources/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/jonmmease/avenger)"
        ),
        Duration::from_secs(30),
    )
    .expect("the fetcher builds");
    let cache = ImageCache::with_options(ImageCacheOptions {
        fetcher: Arc::new(fetcher),
        ..Default::default()
    });
    let engine = avenger_typst_label::bundled_label_engine();
    let avenger_app = AvengerApp::builder(State, Images, engine)
        .size(size())
        .build()
        .await
        .expect("the app builds")
        .with_images(cache);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the runtime builds");
    let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, runtime);
    event_loop.run_app(&mut app).expect("the event loop runs");
}

fn main() {
    pollster::block_on(run());
}
