use avenger_common::value::ScalarOrArray;
use avenger_render_fixtures::resources::TestResolver;
fn gpu_device() -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .unwrap();
        println!("GPU adapter: {:?}", adapter.get_info());
        adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .unwrap()
    })
}
fn pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &avenger_wgpu::offscreen::OffscreenTarget,
) -> image::RgbaImage {
    let stride = (target.extent.width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (stride * target.extent.height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(target.extent.height),
            },
        },
        target.extent,
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = buffer.slice(..).get_mapped_range();
    let data = mapped
        .chunks(stride as usize)
        .flat_map(|row| row[..target.extent.width as usize * 4].to_vec())
        .collect();
    image::RgbaImage::from_vec(target.extent.width, target.extent.height, data).unwrap()
}

/// Draw `scene` with a new renderer from `context` into a new offscreen target that matches the
/// renderer, cleared to white, and read the target back.
fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    context: &avenger_wgpu::context::AvengerGpuContext,
    config: avenger_wgpu::renderer::AvengerRendererConfig,
    scene: &avenger_scenegraph::scene_graph::SceneGraph,
    engine: &avenger_typst_label::LabelEngine,
) -> image::RgbaImage {
    use avenger_wgpu::offscreen::{OffscreenTarget, OffscreenTargetDescriptor};
    let target = OffscreenTarget::new(
        device,
        &OffscreenTargetDescriptor::new(config.dimensions, config.texture_format)
            .with_sample_count(config.sample_count),
    );
    let mut renderer = avenger_wgpu::renderer::AvengerWgpuRenderer::new(context, config);
    renderer.set_scene(device, queue, scene, engine).unwrap();
    renderer
        .render_to_offscreen(device, queue, &target)
        .unwrap();
    pixels(device, queue, &target)
}

/// Record a pass that clears `view` to `color`.
fn clear_pass(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, color: wgpu::Color) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
    });
}
#[test]
fn renderer_resize_preserves_logical_mark_coordinates() {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::{
        offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
        renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
    };
    let (device, queue) = gpu_device();
    let small = CanvasDimensions {
        size: [100.0, 100.0],
        scale: 1.0,
    };
    let large = CanvasDimensions {
        size: [200.0, 100.0],
        scale: 2.0,
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let rect = avenger_scenegraph::marks::rect::SceneRectMark {
        x: ScalarOrArray::new_scalar(10.0),
        y: ScalarOrArray::new_scalar(10.0),
        width: Some(ScalarOrArray::new_scalar(20.0)),
        height: Some(ScalarOrArray::new_scalar(20.0)),
        fill: ScalarOrArray::new_scalar(avenger_color::ColorOrGradient::Color([
            0.8, 0.8, 0.8, 1.0,
        ])),
        fill_pattern: Some(avenger_scenegraph::marks::pattern::PatternFill {
            anchor: avenger_scenegraph::marks::pattern::PatternAnchor::Scene,
            layers: vec![avenger_scenegraph::marks::pattern::PatternLayer::from(
                avenger_scenegraph::marks::pattern::StripeLayer::new(45.0, 8.0, 2.0),
            )],
            ..Default::default()
        })
        .into(),
        clip: true,
        ..Default::default()
    };
    let mut clip = lyon::path::Path::builder();
    clip.begin(lyon::math::point(10.0, 10.0));
    clip.line_to(lyon::math::point(30.0, 10.0));
    clip.line_to(lyon::math::point(10.0, 30.0));
    clip.close();
    let scene = avenger_scenegraph::scene_graph::SceneGraph {
        marks: vec![
            avenger_scenegraph::marks::group::SceneGroup {
                marks: vec![rect.into()],
                clip: avenger_scenegraph::marks::group::Clip::Path(clip.build()),
                ..Default::default()
            }
            .into(),
            avenger_scenegraph::marks::text::SceneTextMark {
                text: ScalarOrArray::new_scalar("Resize $x^2$".to_string()),
                text_syntax: avenger_common::types::TextSyntaxMode::TypstMarkup,
                x: ScalarOrArray::new_scalar(5.0),
                y: ScalarOrArray::new_scalar(50.0),
                ..Default::default()
            }
            .into(),
            avenger_scenegraph::marks::symbol::SceneSymbolMark {
                len: 100,
                x: ScalarOrArray::new_array((0..100).map(|i| 5.0 + i as f32).collect()),
                y: ScalarOrArray::new_scalar(80.0),
                ..Default::default()
            }
            .into(),
        ],
        width: 100.0,
        height: 100.0,
        origin: [0.0, 0.0],
    };
    let mut reused = AvengerWgpuRenderer::new(
        &avenger_wgpu::context::AvengerGpuContext::new(&device),
        AvengerRendererConfig::new(small, format),
    );
    let engine = avenger_typst_label::bundled_label_engine();
    reused.set_scene(&device, &queue, &scene, &engine).unwrap();
    reused.resize(large);
    let target = OffscreenTarget::new(&device, &OffscreenTargetDescriptor::new(large, format));
    reused
        .render_to_offscreen(&device, &queue, &target)
        .unwrap();
    let resized = pixels(&device, &queue, &target);
    let mut fresh = AvengerWgpuRenderer::new(
        &avenger_wgpu::context::AvengerGpuContext::new(&device),
        AvengerRendererConfig::new(large, format),
    );
    fresh.set_scene(&device, &queue, &scene, &engine).unwrap();
    fresh.render_to_offscreen(&device, &queue, &target).unwrap();
    let expected = pixels(&device, &queue, &target);
    assert!(
        resized == expected,
        "resize without replacing scene changed logical coordinates"
    );
}

/// A renderer draws text with the engine that its scene was installed with.
#[test]
fn the_scene_engine_controls_the_rendered_font() {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_scenegraph::{marks::text::SceneTextMark, scene_graph::SceneGraph};
    use avenger_typst_label::{bundled_label_engine, EngineOptions, FontOptions, LabelEngine};
    use avenger_wgpu::{context::AvengerGpuContext, renderer::AvengerRendererConfig};
    let (device, queue) = gpu_device();
    let engine = LabelEngine::new(EngineOptions {
        fonts: FontOptions {
            load_system_fonts: false,
            default_sans_serif_family: Some("DejaVu Sans Mono".to_string()),
            ..avenger_typst_label::bundled_font_options()
        },
    });
    let render = |font: &str, engine: &LabelEngine| {
        let scene = SceneGraph {
            marks: vec![SceneTextMark {
                text: "iiiiWWWW".to_string().into(),
                font: font.to_string().into(),
                font_size: 20.0.into(),
                x: 20.0.into(),
                y: 40.0.into(),
                ..Default::default()
            }
            .into()],
            width: 150.0,
            height: 75.0,
            origin: [0.0, 0.0],
        };
        let config = AvengerRendererConfig::new(
            CanvasDimensions {
                size: [150.0, 75.0],
                scale: 2.0,
            },
            wgpu::TextureFormat::Rgba8Unorm,
        );
        draw(
            &device,
            &queue,
            &AvengerGpuContext::new(&device),
            config,
            &scene,
            engine,
        )
    };
    let injected = render("sans-serif", &engine);
    assert!(
        injected == render("DejaVu Sans Mono", &bundled_label_engine()),
        "the injected engine draws its default sans-serif family"
    );
    assert!(
        injected != render("sans-serif", &bundled_label_engine()),
        "another engine draws another font"
    );
}

/// A multisampled renderer draws into an offscreen target that resolves its samples, as
/// `PngCanvas` does with the same sample count.
#[test]
fn multisampled_offscreen_target_matches_png_canvas() {
    use avenger_color::ColorOrGradient;
    use avenger_common::canvas::CanvasDimensions;
    use avenger_scenegraph::{marks::symbol::SceneSymbolMark, scene_graph::SceneGraph};
    use avenger_wgpu::{
        canvas::{Canvas, CanvasConfig, PngCanvas},
        context::AvengerGpuContext,
        renderer::AvengerRendererConfig,
    };
    let dimensions = CanvasDimensions {
        size: [80.0, 60.0],
        scale: 2.0,
    };
    let scene = SceneGraph {
        width: 80.0,
        height: 60.0,
        origin: [0.0, 0.0],
        marks: vec![SceneSymbolMark {
            len: 3,
            x: vec![20.0, 40.0, 60.0].into(),
            y: 30.0.into(),
            size: 300.0.into(),
            fill: ColorOrGradient::Color([0.2, 0.4, 0.8, 0.7]).into(),
            ..Default::default()
        }
        .into()],
    };
    let engine = avenger_typst_label::bundled_label_engine();
    let mut canvas = pollster::block_on(PngCanvas::new(
        dimensions,
        CanvasConfig {
            sample_count: Some(4),
            ..Default::default()
        },
    ))
    .unwrap();
    canvas.set_scene(&scene, &engine).unwrap();
    let expected = pollster::block_on(canvas.render()).unwrap();

    let (device, queue) = gpu_device();
    let config = AvengerRendererConfig::new(dimensions, wgpu::TextureFormat::Rgba8Unorm)
        .with_sample_count(4);
    let context = AvengerGpuContext::new(&device);
    assert!(draw(&device, &queue, &context, config, &scene, &engine) == expected);
}

/// A renderer drawing into an sRGB target stores the same opaque colors as one drawing into a
/// target without sRGB encoding.
#[test]
fn srgb_target_stores_the_same_opaque_colors() {
    use avenger_color::ColorOrGradient;
    use avenger_common::canvas::CanvasDimensions;
    use avenger_scenegraph::{
        marks::{rect::SceneRectMark, symbol::SceneSymbolMark},
        scene_graph::SceneGraph,
    };
    use avenger_wgpu::{context::AvengerGpuContext, renderer::AvengerRendererConfig};
    let (device, queue) = gpu_device();
    let dimensions = CanvasDimensions {
        size: [90.0, 60.0],
        scale: 1.0,
    };
    let colors = [
        [0.9, 0.2, 0.1, 1.0],
        [0.5, 0.5, 0.5, 1.0],
        [0.1, 0.3, 0.8, 1.0],
    ];
    let scene = SceneGraph {
        width: 90.0,
        height: 60.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneRectMark {
                len: 3,
                x: vec![0.0, 30.0, 60.0].into(),
                y: 0.0.into(),
                width: Some(30.0.into()),
                height: Some(30.0.into()),
                fill: colors
                    .iter()
                    .map(|color| ColorOrGradient::Color(*color))
                    .collect::<Vec<_>>()
                    .into(),
                ..Default::default()
            }
            .into(),
            // 100 symbols draw instanced. All but the first sit off the canvas.
            SceneSymbolMark {
                len: 100,
                x: (0..100)
                    .map(|i| if i == 0 { 45.0 } else { -100.0 })
                    .collect::<Vec<_>>()
                    .into(),
                y: 45.0.into(),
                size: 200.0.into(),
                fill: ColorOrGradient::Color([0.2, 0.6, 0.3, 1.0]).into(),
                ..Default::default()
            }
            .into(),
        ],
    };
    let engine = avenger_typst_label::bundled_label_engine();
    let render = |format: wgpu::TextureFormat| {
        draw(
            &device,
            &queue,
            &AvengerGpuContext::new(&device),
            AvengerRendererConfig::new(dimensions, format),
            &scene,
            &engine,
        )
    };
    let plain = render(wgpu::TextureFormat::Rgba8Unorm);
    let srgb = render(wgpu::TextureFormat::Rgba8UnormSrgb);
    // The centers of the three rects, the symbol, and the background.
    for (x, y) in [(15, 15), (45, 15), (75, 15), (45, 45), (5, 55)] {
        let (a, b) = (plain.get_pixel(x, y), srgb.get_pixel(x, y));
        assert!(
            a.0.iter().zip(b.0).all(|(a, b)| a.abs_diff(b) <= 1),
            "({x}, {y}): {a:?} and {b:?}"
        );
    }
}

/// A scene with the marks that each draw a different way: a rect clipped by a rectangle, a rect
/// clipped by a path, 100 instanced symbols, and a label.
fn clipped_marks_scene() -> avenger_scenegraph::scene_graph::SceneGraph {
    use avenger_color::ColorOrGradient;
    use avenger_scenegraph::marks::{
        group::{Clip, SceneGroup},
        rect::SceneRectMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
    };
    let rect = |x: f32, color: [f32; 4]| SceneRectMark {
        x: x.into(),
        y: 2.0.into(),
        width: Some(25.0.into()),
        height: Some(20.0.into()),
        fill: ColorOrGradient::Color(color).into(),
        clip: true,
        ..Default::default()
    };
    let mut triangle = lyon::path::Path::builder();
    triangle.begin(lyon::math::point(30.0, 0.0));
    triangle.line_to(lyon::math::point(60.0, 0.0));
    triangle.line_to(lyon::math::point(30.0, 30.0));
    triangle.close();
    avenger_scenegraph::scene_graph::SceneGraph {
        width: 60.0,
        height: 40.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneGroup {
                clip: Clip::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 15.0,
                },
                marks: vec![rect(2.0, [0.8, 0.2, 0.2, 1.0]).into()],
                ..Default::default()
            }
            .into(),
            SceneGroup {
                clip: Clip::Path(triangle.build()),
                marks: vec![rect(32.0, [0.2, 0.2, 0.8, 1.0]).into()],
                ..Default::default()
            }
            .into(),
            SceneSymbolMark {
                len: 100,
                x: (0..100)
                    .map(|i| (i % 20) as f32 * 3.0)
                    .collect::<Vec<_>>()
                    .into(),
                y: (0..100)
                    .map(|i| 28.0 + (i / 20) as f32 * 2.0)
                    .collect::<Vec<_>>()
                    .into(),
                size: 4.0.into(),
                fill: ColorOrGradient::Color([0.1, 0.6, 0.3, 0.8]).into(),
                ..Default::default()
            }
            .into(),
            SceneTextMark {
                text: "Ag".to_string().into(),
                x: 45.0.into(),
                y: 38.0.into(),
                font_size: 10.0.into(),
                ..Default::default()
            }
            .into(),
        ],
    }
}

/// Recording into the caller's encoder draws what `render_to_offscreen` draws.
#[test]
fn encode_into_caller_encoder_matches_render_to_offscreen() {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::{
        offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
        renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
        target::WHITE_CLEAR,
    };
    let (device, queue) = gpu_device();
    let dimensions = CanvasDimensions {
        size: [60.0, 40.0],
        scale: 2.0,
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = clipped_marks_scene();
    let engine = avenger_typst_label::bundled_label_engine();
    let mut renderer = AvengerWgpuRenderer::new(
        &avenger_wgpu::context::AvengerGpuContext::new(&device),
        AvengerRendererConfig::new(dimensions, format),
    );
    renderer
        .set_scene(&device, &queue, &scene, &engine)
        .unwrap();
    let target = OffscreenTarget::new(&device, &OffscreenTargetDescriptor::new(dimensions, format));

    renderer
        .render_to_offscreen(&device, &queue, &target)
        .unwrap();
    let expected = pixels(&device, &queue, &target);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    renderer
        .encode(
            &device,
            &queue,
            &mut encoder,
            target.render_target(WHITE_CLEAR),
        )
        .unwrap();
    queue.submit([encoder.finish()]);
    assert!(pixels(&device, &queue, &target) == expected);
}

/// A target with a viewport origin draws the scene into the rectangle the renderer's size at that
/// origin, as it draws the scene into a target of its own, and leaves the rest of the attachment.
/// At scale 2, clip rectangles and the clip stencil follow the origin in physical pixels.
#[test]
fn viewport_origin_draws_the_scene_into_its_rectangle() {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::{
        offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
        renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
    };
    let (device, queue) = gpu_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = clipped_marks_scene();
    let engine = avenger_typst_label::bundled_label_engine();
    for scale in [1.0, 2.0] {
        let dimensions = CanvasDimensions {
            size: [60.0, 40.0],
            scale,
        };
        let mut renderer = AvengerWgpuRenderer::new(
            &avenger_wgpu::context::AvengerGpuContext::new(&device),
            AvengerRendererConfig::new(dimensions, format),
        );
        renderer
            .set_scene(&device, &queue, &scene, &engine)
            .unwrap();

        let own =
            OffscreenTarget::new(&device, &OffscreenTargetDescriptor::new(dimensions, format));
        let commands = renderer
            .build_frame_commands(
                &device,
                &queue,
                own.render_target(wgpu::LoadOp::Clear(wgpu::Color::GREEN)),
                None,
            )
            .unwrap();
        queue.submit(commands);
        let expected = pixels(&device, &queue, &own);

        let larger = OffscreenTarget::new(
            &device,
            &OffscreenTargetDescriptor::new(
                CanvasDimensions {
                    size: [120.0, 90.0],
                    scale,
                },
                format,
            ),
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        clear_pass(&mut encoder, &larger.view, wgpu::Color::GREEN);
        let origin = [30, 25];
        renderer
            .encode(
                &device,
                &queue,
                &mut encoder,
                larger
                    .render_target(wgpu::LoadOp::Load)
                    .with_viewport_origin(origin),
            )
            .unwrap();
        queue.submit([encoder.finish()]);
        let (width, height) = expected.dimensions();
        for (x, y, pixel) in pixels(&device, &queue, &larger).enumerate_pixels() {
            let inside = (origin[0]..origin[0] + width).contains(&x)
                && (origin[1]..origin[1] + height).contains(&y);
            let expected = if inside {
                *expected.get_pixel(x - origin[0], y - origin[1])
            } else {
                image::Rgba([0, 255, 0, 255])
            };
            assert_eq!(*pixel, expected, "scale {scale}, pixel ({x}, {y})");
        }
    }
}

/// The renderer can't draw into a target with another format or sample count, a target with a
/// viewport origin that clears, because a clear erases the whole attachment, or a viewport that
/// doesn't fit in the attachment.
#[test]
fn mismatched_targets_are_invalid() {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::{
        error::AvengerWgpuError,
        offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
        renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
        target::WHITE_CLEAR,
    };
    let (device, queue) = gpu_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = AvengerWgpuRenderer::new(
        &avenger_wgpu::context::AvengerGpuContext::new(&device),
        AvengerRendererConfig::new(
            CanvasDimensions {
                size: [60.0, 40.0],
                scale: 1.0,
            },
            format,
        ),
    );
    renderer
        .set_scene(
            &device,
            &queue,
            &clipped_marks_scene(),
            &avenger_typst_label::bundled_label_engine(),
        )
        .unwrap();
    let target = |format, sample_count| {
        OffscreenTarget::new(
            &device,
            &OffscreenTargetDescriptor::new(
                CanvasDimensions {
                    size: [120.0, 90.0],
                    scale: 1.0,
                },
                format,
            )
            .with_sample_count(sample_count),
        )
    };
    let srgb = target(wgpu::TextureFormat::Rgba8UnormSrgb, 1);
    let multisampled = target(format, 4);
    let larger = target(format, 1);
    for target in [
        srgb.render_target(WHITE_CLEAR),
        multisampled.render_target(WHITE_CLEAR),
        larger
            .render_target(WHITE_CLEAR)
            .with_viewport_origin([30, 25]),
        larger
            .render_target(wgpu::LoadOp::Load)
            .with_viewport_origin([70, 60]),
    ] {
        assert!(matches!(
            renderer.build_frame_commands(&device, &queue, target, None),
            Err(AvengerWgpuError::InvalidTarget(_))
        ));
    }
}

/// Marks drawn onto a transparent clear store premultiplied colors: each channel multiplied by
/// the pixel's alpha.
#[test]
fn transparent_clear_stores_premultiplied_colors() {
    use avenger_color::ColorOrGradient;
    use avenger_common::canvas::CanvasDimensions;
    use avenger_scenegraph::{marks::rect::SceneRectMark, scene_graph::SceneGraph};
    use avenger_wgpu::{
        context::AvengerGpuContext,
        offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
        renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
        target::TRANSPARENT_CLEAR,
    };
    let (device, queue) = gpu_device();
    let dimensions = CanvasDimensions {
        size: [20.0, 20.0],
        scale: 1.0,
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = SceneGraph {
        width: 20.0,
        height: 20.0,
        origin: [0.0, 0.0],
        marks: vec![SceneRectMark {
            x: 0.0.into(),
            y: 0.0.into(),
            width: Some(20.0.into()),
            height: Some(20.0.into()),
            fill: ColorOrGradient::Color([0.8, 0.4, 0.2, 0.5]).into(),
            ..Default::default()
        }
        .into()],
    };
    let mut renderer = AvengerWgpuRenderer::new(
        &AvengerGpuContext::new(&device),
        AvengerRendererConfig::new(dimensions, format),
    );
    renderer
        .set_scene(
            &device,
            &queue,
            &scene,
            &avenger_typst_label::bundled_label_engine(),
        )
        .unwrap();
    let target = OffscreenTarget::new(&device, &OffscreenTargetDescriptor::new(dimensions, format));
    let commands = renderer
        .build_frame_commands(
            &device,
            &queue,
            target.render_target(TRANSPARENT_CLEAR),
            None,
        )
        .unwrap();
    queue.submit(commands);
    let pixel = *pixels(&device, &queue, &target).get_pixel(10, 10);
    for (channel, expected) in pixel.0.iter().zip([102u8, 51, 26, 128]) {
        assert!(channel.abs_diff(expected) <= 1, "{pixel:?}");
    }
}

/// Two renderers from one context record two scenes into one texture, each into its own viewport,
/// in one encoder that is submitted once. Each viewport holds what its scene draws with a renderer
/// of its own.
#[test]
fn renderers_sharing_a_context_draw_into_one_texture() {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_scenegraph::marks::group::SceneGroup;
    use avenger_wgpu::{
        context::AvengerGpuContext,
        offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
        renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
    };
    let (device, queue) = gpu_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let engine = avenger_typst_label::bundled_label_engine();
    // The second scene draws the same marks shifted, so the two renderers hold different buffers.
    let mut shifted = clipped_marks_scene();
    shifted.marks = vec![SceneGroup {
        origin: [6.0, 4.0],
        marks: shifted.marks,
        ..Default::default()
    }
    .into()];
    let scenes = [clipped_marks_scene(), shifted];
    let context = AvengerGpuContext::new(&device);
    for scale in [1.0, 2.0] {
        let dimensions = CanvasDimensions {
            size: [60.0, 40.0],
            scale,
        };
        let texture = OffscreenTarget::new(
            &device,
            &OffscreenTargetDescriptor::new(
                CanvasDimensions {
                    size: [120.0, 40.0],
                    scale,
                },
                format,
            ),
        );
        let width = dimensions.to_physical_width();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        clear_pass(&mut encoder, &texture.view, wgpu::Color::WHITE);
        let mut renderers = Vec::new();
        for (index, scene) in scenes.iter().enumerate() {
            let mut renderer =
                AvengerWgpuRenderer::new(&context, AvengerRendererConfig::new(dimensions, format));
            renderer.set_scene(&device, &queue, scene, &engine).unwrap();
            renderer
                .encode(
                    &device,
                    &queue,
                    &mut encoder,
                    texture
                        .render_target(wgpu::LoadOp::Load)
                        .with_viewport_origin([index as u32 * width, 0]),
                )
                .unwrap();
            renderers.push(renderer);
        }
        queue.submit([encoder.finish()]);
        let actual = pixels(&device, &queue, &texture);
        for (index, scene) in scenes.iter().enumerate() {
            let alone = draw(
                &device,
                &queue,
                &AvengerGpuContext::new(&device),
                AvengerRendererConfig::new(dimensions, format),
                scene,
                &engine,
            );
            for (x, y, pixel) in alone.enumerate_pixels() {
                assert_eq!(
                    actual.get_pixel(index as u32 * width + x, y),
                    pixel,
                    "scale {scale}, scene {index}, pixel ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn image_filtering_preserves_partial_alpha_on_transparent_and_opaque_targets() {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_image::RgbaImage;
    use avenger_scenegraph::{
        marks::image::{SceneImageMark, SceneImageSource},
        scene_graph::SceneGraph,
    };
    use avenger_wgpu::{
        offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
        renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
    };
    let (device, queue) = gpu_device();
    let dimensions = CanvasDimensions {
        size: [8.0, 8.0],
        scale: 1.0,
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = OffscreenTarget::new(&device, &OffscreenTargetDescriptor::new(dimensions, format));
    let mut renderer = AvengerWgpuRenderer::new(
        &avenger_wgpu::context::AvengerGpuContext::new(&device),
        AvengerRendererConfig::new(dimensions, format),
    );
    for smooth in [false, true] {
        let scene = SceneGraph {
            width: 8.0,
            height: 8.0,
            origin: [0.0; 2],
            marks: vec![SceneImageMark {
                image: SceneImageSource::from(RgbaImage {
                    width: 2,
                    height: 1,
                    data: vec![200, 40, 100, 128, 40, 200, 100, 64],
                })
                .into(),
                width: 8.0.into(),
                height: 8.0.into(),
                aspect: false,
                smooth,
                ..Default::default()
            }
            .into()],
        };
        renderer
            .set_scene(
                &device,
                &queue,
                &scene,
                &avenger_typst_label::bundled_label_engine(),
            )
            .unwrap();
        for clear in [
            wgpu::Color::TRANSPARENT,
            wgpu::Color::BLACK,
            wgpu::Color::WHITE,
        ] {
            queue.submit(
                renderer
                    .build_frame_commands(
                        &device,
                        &queue,
                        target.render_target(wgpu::LoadOp::Clear(clear)),
                        None,
                    )
                    .unwrap(),
            );
            let actual = pixels(&device, &queue, &target).get_pixel(3, 4).0;
            // At this pixel linear filtering weights the two premultiplied texels 5:3.
            let mut expected = if smooth {
                [66, 31, 41, 104]
            } else {
                [100, 20, 50, 128]
            };
            if clear.a == 1.0 {
                if clear.r == 1.0 {
                    let remaining = 255 - expected[3];
                    for color in &mut expected[..3] {
                        *color += remaining;
                    }
                }
                expected[3] = 255;
            }
            assert!(
                actual
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(b) <= 1),
                "smooth={smooth}, clear={clear:?}: {actual:?} != {expected:?}"
            );
        }
    }
}

fn resource_canvas(resolver: &std::sync::Arc<TestResolver>) -> avenger_wgpu::canvas::PngCanvas {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};
    pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [20.0, 10.0],
            scale: 1.0,
        },
        CanvasConfig {
            images: Some(resolver.clone()),
            ..Default::default()
        },
    ))
    .unwrap()
}

/// Resources draw their placeholders until the resolver has their images, and each image draws
/// on the frame after it arrives, without a new scene. Frames whose answers don't change neither
/// install the scene nor rebuild its atlases.
#[test]
fn resource_images_draw_as_they_arrive() {
    use avenger_image::ImageState;
    use avenger_render_fixtures::resources::{
        pixel, resource_scene, IMAGE_URL, MESH_URL, PLACEHOLDER,
    };
    use avenger_wgpu::canvas::Canvas;
    let resolver = std::sync::Arc::new(TestResolver::default());
    let mut canvas = resource_canvas(&resolver);
    canvas
        .set_scene(
            &resource_scene(),
            &avenger_typst_label::bundled_label_engine(),
        )
        .unwrap();
    let mut frame = || {
        let image = pollster::block_on(canvas.render()).unwrap();
        let colors = [image.get_pixel(5, 5).0, image.get_pixel(15, 5).0];
        (colors, canvas.frame_stats(), canvas.image_status())
    };
    let (red, blue) = ([255, 0, 0, 255], [0, 0, 255, 255]);

    let (colors, stats, status) = frame();
    assert_eq!(colors, [PLACEHOLDER, PLACEHOLDER]);
    assert!(stats.installed && stats.prepared);
    assert_eq!(status.pending, [IMAGE_URL, MESH_URL]);

    resolver.set(IMAGE_URL, ImageState::Ready(pixel(red)));
    let (colors, stats, _) = frame();
    assert_eq!(colors, [red, PLACEHOLDER]);
    assert!(stats.installed);

    resolver.set(MESH_URL, ImageState::Ready(pixel(blue)));
    let (colors, _, status) = frame();
    assert_eq!(colors, [red, blue]);
    assert!(status.is_complete());

    let (colors, stats, _) = frame();
    assert_eq!(colors, [red, blue]);
    assert!(!stats.installed && !stats.prepared, "{stats:?}");

    resolver.set(IMAGE_URL, ImageState::Failed("gone".into()));
    let (colors, _, status) = frame();
    assert_eq!(colors, [PLACEHOLDER, blue]);
    assert_eq!(status.failed.len(), 1);
}

/// A canvas leases its scene's images until another scene replaces it or the canvas drops, and it
/// takes the new scene's lease before it releases the old one's.
#[test]
fn a_scene_lease_lasts_until_the_scene_is_replaced_or_the_canvas_drops() {
    use avenger_render_fixtures::resources::{resource_scene, IMAGE_URL, MESH_URL};
    use avenger_scenegraph::marks::mark::SceneMark;
    use avenger_wgpu::canvas::Canvas;
    let engine = avenger_typst_label::bundled_label_engine();
    let resolver = std::sync::Arc::new(TestResolver::default());
    let mut canvas = resource_canvas(&resolver);
    let scene = resource_scene();
    canvas.set_scene(&scene, &engine).unwrap();
    assert_eq!((resolver.held(IMAGE_URL), resolver.held(MESH_URL)), (1, 1));

    // The next scene keeps only the mesh image.
    let mut mesh_only = scene.clone();
    mesh_only
        .marks
        .retain(|mark| matches!(mark, SceneMark::MeshImage(_)));
    canvas.set_scene(&mesh_only, &engine).unwrap();
    pollster::block_on(canvas.render()).unwrap();
    assert_eq!((resolver.held(IMAGE_URL), resolver.held(MESH_URL)), (0, 1));
    assert_eq!(resolver.released(), [IMAGE_URL]);

    drop(canvas);
    assert_eq!(resolver.held(MESH_URL), 0);
}
