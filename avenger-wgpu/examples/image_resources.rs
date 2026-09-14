//! Render pending, ready, and warped resource images without a network request.
use std::sync::Arc;

use avenger_common::canvas::CanvasDimensions;
use avenger_image::{ImageResourceResolver, ImageResourceState, RgbaImage};
use avenger_resource::ResourceKey;
use avenger_scenegraph::{
    marks::{
        image::{
            SceneImageMark, SceneImageResource, SceneImageSource, SceneImageUnavailablePolicy,
        },
        text::SceneTextMark,
        warped_image::SceneWarpedImageMark,
    },
    scene_graph::SceneGraph,
};
use avenger_wgpu::{
    canvas::{Canvas, CanvasConfig, PngCanvas},
    image_resources::{WgpuImagePlaceholder, WgpuImageResourceConfig, WgpuMissingImagePolicy},
};

struct ExampleImages(Arc<RgbaImage>);

impl ImageResourceResolver for ExampleImages {
    fn image_state(&self, key: &ResourceKey) -> ImageResourceState {
        match key.0.as_str() {
            "grid" => ImageResourceState::Ready(self.0.clone()),
            "loading" => ImageResourceState::Pending,
            _ => ImageResourceState::Missing,
        }
    }
}

fn grid_image() -> RgbaImage {
    let mut data = Vec::with_capacity(128 * 128 * 4);
    for y in 0..128 {
        for x in 0..128 {
            let pixel = if x % 16 == 0 || y % 16 == 0 {
                [245, 250, 255, 255]
            } else {
                [40 + x as u8, 90 + y as u8, 195 - (x / 2) as u8, 255]
            };
            data.extend(pixel);
        }
    }
    RgbaImage {
        width: 128,
        height: 128,
        data,
    }
}

fn source(key: &str) -> SceneImageSource {
    SceneImageSource::Resource(SceneImageResource {
        key: key.into(),
        intrinsic_width: 128,
        intrinsic_height: 128,
        fallback_key: None,
    })
}

fn label(text: &str, x: f32, y: f32, font_size: f32) -> SceneTextMark {
    SceneTextMark {
        text: text.to_string().into(),
        x: x.into(),
        y: y.into(),
        font_size: font_size.into(),
        ..Default::default()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "image-resources.png".into());
    let mut marks = vec![
        label("Images supplied by the application", 24.0, 40.0, 24.0).into(),
        label("Pending resource", 24.0, 88.0, 16.0).into(),
        label("Ready resource", 254.0, 88.0, 16.0).into(),
        label("Same image on a curved mesh", 484.0, 88.0, 16.0).into(),
    ];
    for (key, x) in [("loading", 24.0), ("grid", 254.0)] {
        marks.push(
            SceneImageMark {
                image: vec![source(key)].into(),
                x: x.into(),
                y: 108.0.into(),
                width: 200.0.into(),
                height: 200.0.into(),
                aspect: false,
                unavailable_policy: SceneImageUnavailablePolicy::RendererDefault,
                ..Default::default()
            }
            .into(),
        );
    }
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for row in 0..=16 {
        for col in 0..=16 {
            let (u, v) = (col as f32 / 16.0, row as f32 / 16.0);
            positions.push([
                484.0 + 200.0 * u + 35.0 * (v * std::f32::consts::PI).sin(),
                128.0 + 180.0 * v - 20.0 * (u * std::f32::consts::PI).sin(),
            ]);
            uvs.push([u, v]);
            if row < 16 && col < 16 {
                let i = row * 17 + col;
                indices.extend([i, i + 1, i + 18, i, i + 18, i + 17]);
            }
        }
    }
    marks.push(
        SceneWarpedImageMark {
            image: source("grid"),
            positions,
            uvs,
            indices,
            ..Default::default()
        }
        .into(),
    );
    let scene = SceneGraph {
        width: 770.0,
        height: 340.0,
        origin: [0.0; 2],
        marks,
    };
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig {
            image_resource_config: WgpuImageResourceConfig {
                resolver: Some(Arc::new(ExampleImages(Arc::new(grid_image())))),
                missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
                placeholder: WgpuImagePlaceholder::Solid([224, 231, 236, 255]),
            },
            ..Default::default()
        },
    ))?;
    canvas.set_scene(&scene)?;
    pollster::block_on(canvas.render())?.save(&output)?;
    println!("Saved {output}");
    Ok(())
}
