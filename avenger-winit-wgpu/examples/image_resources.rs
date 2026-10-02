//! Present a delayed local image and resize its canvas independently of the window.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use async_trait::async_trait;
    use avenger_app::{
        app::{AvengerApp, SceneGraphBuilder},
        error::AvengerAppError,
    };
    use avenger_eventstream::{
        manager::EventStreamHandler,
        scene::{SceneGraphEvent, SceneGraphEventType},
        stream::{EventStreamConfig, UpdateStatus},
    };
    use avenger_geometry::rtree::SceneGraphRTree;
    use avenger_image::{error::AvengerImageError, fetcher::ImageFetcher, ImageResourceCache};
    use avenger_resource::{RenderInvalidationHub, ResourceRequest, ResourceSource};
    use avenger_scenegraph::{
        marks::{
            image::{SceneImageMark, SceneImageResource, SceneImageSource},
            text::SceneTextMark,
        },
        scene_graph::SceneGraph,
    };
    use avenger_winit_wgpu::{
        CanvasConfig, CanvasFrameOptions, WgpuImageResourceConfig, WgpuMissingImagePolicy,
        WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    };
    use std::{error::Error, sync::Arc, time::Duration};
    use winit::window::WindowAttributes;

    struct LocalGrid;
    impl ImageFetcher for LocalGrid {
        fn fetch_image(&self, _: &str) -> Result<image::DynamicImage, AvengerImageError> {
            std::thread::sleep(Duration::from_millis(1500));
            Ok(image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(
                128,
                128,
                |x, y| {
                    image::Rgba(if x % 16 == 0 || y % 16 == 0 {
                        [245, 250, 255, 255]
                    } else {
                        [40 + x as u8, 90 + y as u8, 195 - (x / 2) as u8, 255]
                    })
                },
            )))
        }
    }

    struct Builder;
    #[async_trait]
    impl SceneGraphBuilder<[f32; 2]> for Builder {
        async fn build(&self, size: &mut [f32; 2]) -> Result<SceneGraph, AvengerAppError> {
            Ok(SceneGraph {
                width: size[0],
                height: size[1],
                origin: [0.0; 2],
                marks: vec![
                    SceneTextMark {
                        text: "A local image arrives after 1.5 seconds".to_string().into(),
                        x: 20.0.into(),
                        y: 30.0.into(),
                        font_size: 16.0.into(),
                        ..Default::default()
                    }
                    .into(),
                    SceneImageMark {
                        image: vec![SceneImageSource::Resource(SceneImageResource {
                            key: "grid".into(),
                            intrinsic_width: 128,
                            intrinsic_height: 128,
                            fallback_key: None,
                        })]
                        .into(),
                        x: 20.0.into(),
                        y: 50.0.into(),
                        width: (size[0] - 40.0).into(),
                        height: (size[1] - 70.0).into(),
                        aspect: false,
                        ..Default::default()
                    }
                    .into(),
                ],
            })
        }
    }

    struct Resize;
    #[async_trait]
    impl EventStreamHandler<[f32; 2]> for Resize {
        async fn handle(
            &self,
            event: &SceneGraphEvent,
            size: &mut [f32; 2],
            _: &SceneGraphRTree,
        ) -> UpdateStatus {
            if let SceneGraphEvent::CanvasResize(event) = event {
                *size = event.size;
            }
            UpdateStatus {
                rerender: true,
                rebuild_geometry: true,
                ..Default::default()
            }
        }
    }

    pub fn run() -> Result<(), Box<dyn Error>> {
        let hub = RenderInvalidationHub::default();
        let images = Arc::new(
            ImageResourceCache::with_fetcher(Arc::new(LocalGrid))
                .with_render_invalidation_sink(Arc::new(hub.clone())),
        );
        images.request(ResourceRequest::new(
            "grid".into(),
            "image".into(),
            ResourceSource::Url {
                // The supplied fetcher handles this URL locally.
                url: "https://example.invalid/grid.png".into(),
            },
        ));
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let app = runtime.block_on(AvengerApp::try_new(
            [420.0, 310.0],
            Arc::new(Builder),
            vec![(
                EventStreamConfig {
                    types: vec![SceneGraphEventType::CanvasResize],
                    ..Default::default()
                },
                Arc::new(Resize),
            )],
        ))?;
        let options = WinitWgpuAvengerAppOptions::new(2.0)
            .window_attributes(WindowAttributes::default().with_title("Image resources"))
            .canvas_frame(Some(CanvasFrameOptions {
                resize_width: true,
                resize_height: true,
                min_size: [360.0, 200.0],
                extra_window_size: [150.0, 120.0],
                ..Default::default()
            }))
            .canvas_config(CanvasConfig {
                image_resource_config: WgpuImageResourceConfig {
                    resolver: Some(images),
                    missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
                    ..Default::default()
                },
                ..Default::default()
            })
            .render_invalidation_hub(hub);
        let (mut host, event_loop) =
            WinitWgpuAvengerApp::try_new_and_event_loop_with_options(app, options, runtime)?;
        event_loop.run_app(&mut host)?;
        if let Some(error) = host.take_fatal_error() {
            return Err(error.into());
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    native::run()
}
#[cfg(target_arch = "wasm32")]
fn main() {}
