use avenger_common::value::ScalarOrArray;
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
            anchor: avenger_scenegraph::marks::pattern::PatternAnchor::Chart,
            layers: vec![avenger_scenegraph::marks::pattern::PatternLayer::Stripe(
                avenger_scenegraph::marks::pattern::StripePatternLayer::new(45.0, 8.0, 2.0),
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
                text_syntax: avenger_text::types::TextSyntaxMode::TypstMarkup,
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
    let mut reused = AvengerWgpuRenderer::new(&device, AvengerRendererConfig::new(small, format));
    reused.set_scene(&device, &queue, &scene).unwrap();
    reused.resize(large);
    let mut target =
        OffscreenTarget::new(&device, &OffscreenTargetDescriptor::new(large, format), 1);
    reused
        .render_to_offscreen(&device, &queue, &mut target)
        .unwrap();
    let resized = pixels(&device, &queue, &target);
    let mut fresh = AvengerWgpuRenderer::new(&device, AvengerRendererConfig::new(large, format));
    fresh.set_scene(&device, &queue, &scene).unwrap();
    fresh
        .render_to_offscreen(&device, &queue, &mut target)
        .unwrap();
    let expected = pixels(&device, &queue, &target);
    assert!(
        resized == expected,
        "resize without replacing scene changed logical coordinates"
    );
}

#[test]
fn configured_text_engine_controls_the_rendered_font_and_rich_limit() {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_scenegraph::{marks::text::SceneTextMark, scene_graph::SceneGraph};
    use avenger_text::{types::TextSyntaxMode, FontResolutionOptions, LabelParamValue, TextEngine};
    use avenger_wgpu::{
        canvas::CanvasConfig,
        offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
        renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
    };
    let (device, queue) = gpu_device();
    let dimensions = CanvasDimensions {
        size: [150.0, 75.0],
        scale: 2.0,
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut mark = SceneTextMark {
        text: "#label".to_string().into(),
        text_syntax: TextSyntaxMode::TypstMarkup,
        font: "sans-serif".to_string().into(),
        font_size: 20.0.into(),
        x: 20.0.into(),
        y: 40.0.into(),
        limit: 45.0.into(),
        ..Default::default()
    };
    mark.text_params.insert(
        "label".into(),
        LabelParamValue::Str("iiiiWWWW extended label".into()),
    );
    let engine = TextEngine::with_font_resolution(&FontResolutionOptions {
        load_system_fonts: false,
        default_sans_serif_family: Some("DejaVu Sans Mono".to_string()),
        ..avenger_text::default_font_resolution()
    })
    .unwrap();
    let render = |mark: SceneTextMark, config: CanvasConfig| {
        let scene = SceneGraph {
            marks: vec![mark.into()],
            width: 150.0,
            height: 75.0,
            origin: [0.0, 0.0],
        };
        let mut renderer = AvengerWgpuRenderer::new(
            &device,
            AvengerRendererConfig::new(dimensions, format).with_canvas_config(config),
        );
        let mut target = OffscreenTarget::new(
            &device,
            &OffscreenTargetDescriptor::new(dimensions, format),
            1,
        );
        renderer.set_scene(&device, &queue, &scene).unwrap();
        renderer
            .render_to_offscreen(&device, &queue, &mut target)
            .unwrap();
        pixels(&device, &queue, &target)
    };
    let injected = render(
        mark.clone(),
        CanvasConfig {
            text_engine: Some(engine),
            ..Default::default()
        },
    );
    let default = render(mark.clone(), CanvasConfig::default());
    mark.font = "DejaVu Sans Mono".to_string().into();
    let explicit = render(mark, CanvasConfig::default());
    assert!(
        injected == explicit,
        "injected engine must match its resolved font"
    );
    assert!(injected != default, "custom font must change visible text");
    assert!(
        injected
            .enumerate_pixels()
            .all(|(x, _, pixel)| x <= 130 || pixel.0 == [255, 255, 255, 255]),
        "rich text must stop at the logical width limit"
    );
}
