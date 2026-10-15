//! Avenger charts inside an egui application. egui owns the window and the wgpu device. Avenger
//! draws two charts into one texture, each into its own viewport, and egui shows the texture as
//! an image. The charts split the panel's width and take their colors from egui's light or dark
//! visuals. Hovering a mark shows an egui tooltip, picked from the chart's scene.

use avenger_color::ColorOrGradient;
use avenger_common::canvas::CanvasDimensions;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::{
    marks::{rect::SceneRectMark, symbol::SceneSymbolMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_typst_label::LabelEngine;
use avenger_wgpu::{
    context::AvengerGpuContext,
    offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
    renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
    wgpu,
};
use eframe::egui;

/// The height of each chart, in egui points.
const CHART_HEIGHT: f32 = 240.0;
/// The narrowest that each chart gets, in egui points.
const MIN_CHART_WIDTH: f32 = 200.0;
const BINS: usize = 12;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Avenger in egui",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[derive(Clone, Copy, PartialEq)]
struct Settings {
    count: usize,
    size: f32,
    opacity: f32,
}

/// The chart colors, from egui's visuals.
#[derive(Clone, Copy, PartialEq)]
struct Theme {
    plot: [f32; 4],
    border: [f32; 4],
    text: [f32; 4],
    points: [f32; 3],
    bars: [f32; 3],
}

impl Theme {
    fn new(visuals: &egui::Visuals) -> Self {
        let color = |color: egui::Color32| {
            color
                .to_srgba_unmultiplied()
                .map(|channel| f32::from(channel) / 255.0)
        };
        // Lighter marks stand out on a dark plot area.
        let (points, bars) = if visuals.dark_mode {
            ([0.4, 0.68, 1.0], [1.0, 0.6, 0.25])
        } else {
            ([0.13, 0.44, 0.64], [0.85, 0.37, 0.01])
        };
        let text = color(visuals.text_color());
        Self {
            // A faint wash of the text color, which egui's panel shows through.
            plot: [text[0], text[1], text[2], 0.06],
            border: color(visuals.widgets.noninteractive.bg_stroke.color),
            text,
            points,
            bars,
        }
    }
}

struct App {
    settings: Settings,
    drawn: Option<(Settings, Theme)>,
    charts: Charts,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let state = cc
            .wgpu_render_state
            .as_ref()
            .expect("eframe runs with its wgpu renderer");
        Self {
            settings: Settings {
                count: 300,
                size: 40.0,
                opacity: 0.6,
            },
            drawn: None,
            charts: Charts::new(
                state,
                [MIN_CHART_WIDTH, CHART_HEIGHT],
                cc.egui_ctx.pixels_per_point(),
            ),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui::SidePanel::left("settings").show(ctx, |ui| {
            ui.heading("Points");
            ui.add(egui::Slider::new(&mut self.settings.count, 10..=3000).text("count"));
            ui.add(egui::Slider::new(&mut self.settings.size, 4.0..=200.0).text("size"));
            ui.add(egui::Slider::new(&mut self.settings.opacity, 0.05..=1.0).text("opacity"));
            ui.label("From 100 points, the scatter's symbols draw instanced.");
        });

        let theme = Theme::new(&ctx.style().visuals);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Two Avenger charts in one texture, over egui's panel");
            let state = frame
                .wgpu_render_state()
                .expect("eframe runs with its wgpu renderer");
            // The charts split the panel's width, in whole pixels.
            let scale = ctx.pixels_per_point();
            let width = ((ui.available_width() / 2.0 * scale).floor() / scale).max(MIN_CHART_WIDTH);
            let resized = self.charts.resize(state, [width, CHART_HEIGHT], scale);
            if resized || self.drawn != Some((self.settings, theme)) {
                self.charts
                    .draw(&state.device, &state.queue, self.settings, theme);
                self.drawn = Some((self.settings, theme));
            }
            let image = egui::Image::new((
                self.charts.texture_id,
                egui::vec2(2.0 * width, CHART_HEIGHT),
            ))
            .sense(egui::Sense::hover());
            let response = ui.add(image);
            if let Some(position) = response.hover_pos() {
                let local = position - response.rect.min;
                if let Some(tip) = self.charts.describe([local.x, local.y]) {
                    response.on_hover_text_at_pointer(tip);
                }
            }
        });
    }
}

/// Two charts that share one GPU context and draw into one texture.
struct Charts {
    engine: LabelEngine,
    target: OffscreenTarget,
    texture_id: egui::TextureId,
    renderers: [AvengerWgpuRenderer; 2],
    trees: [Option<SceneGraphRTree>; 2],
    points: Vec<[f32; 2]>,
    bins: Vec<usize>,
    /// The size of each chart, in egui points.
    size: [f32; 2],
    /// Pixels per egui point.
    scale: f32,
}

impl Charts {
    fn new(state: &eframe::egui_wgpu::RenderState, size: [f32; 2], scale: f32) -> Self {
        let context = AvengerGpuContext::new(&state.device);
        let chart = CanvasDimensions { size, scale };
        let renderer =
            || AvengerWgpuRenderer::new(&context, AvengerRendererConfig::new(chart, FORMAT));
        let target = chart_texture(&state.device, size, scale);
        let texture_id = state.renderer.write().register_native_texture(
            &state.device,
            &target.view,
            wgpu::FilterMode::Linear,
        );
        Self {
            engine: avenger_typst_label::bundled_label_engine(),
            target,
            texture_id,
            renderers: [renderer(), renderer()],
            trees: [None, None],
            points: Vec::new(),
            bins: Vec::new(),
            size,
            scale,
        }
    }

    /// Resize the charts and their texture, and report whether the size or scale changed.
    fn resize(
        &mut self,
        state: &eframe::egui_wgpu::RenderState,
        size: [f32; 2],
        scale: f32,
    ) -> bool {
        if size == self.size && scale == self.scale {
            return false;
        }
        self.size = size;
        self.scale = scale;
        self.target = chart_texture(&state.device, size, scale);
        state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &state.device,
                &self.target.view,
                wgpu::FilterMode::Linear,
                self.texture_id,
            );
        for renderer in &mut self.renderers {
            renderer.resize(CanvasDimensions { size, scale });
        }
        true
    }

    /// Draw both charts into the texture, in a command encoder that egui's frame follows.
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        settings: Settings,
        theme: Theme,
    ) {
        self.points = points(settings.count);
        self.bins = bins(&self.points);
        let width = self.size[0];
        let scenes = [
            scatter(&self.points, settings, &theme, width),
            histogram(&self.bins, settings, &theme, width),
        ];
        let chart_pixels = CanvasDimensions {
            size: self.size,
            scale: self.scale,
        }
        .to_physical_width();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Avenger charts"),
        });
        // Clear to transparent, so egui's panel shows where the charts draw nothing.
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Clear the chart texture"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.target.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        for (index, scene) in scenes.iter().enumerate() {
            let renderer = &mut self.renderers[index];
            renderer
                .set_scene(device, queue, scene, &self.engine)
                .expect("install the chart");
            let origin = [index as u32 * chart_pixels, 0];
            renderer
                .encode(
                    device,
                    queue,
                    &mut encoder,
                    self.target
                        .render_target(wgpu::LoadOp::Load)
                        .with_viewport_origin(origin),
                )
                .expect("draw the chart");
            self.trees[index] = Some(SceneGraphRTree::from_scene_graph(scene, &self.engine));
        }
        queue.submit([encoder.finish()]);
    }

    /// The tooltip for the mark under `position`, in points from the image's top-left corner.
    fn describe(&self, position: [f32; 2]) -> Option<String> {
        let chart = usize::from(position[0] >= self.size[0]);
        let point = [position[0] - chart as f32 * self.size[0], position[1]];
        let hit = self.trees[chart].as_ref()?.pick_top_mark_at_point(&point)?;
        let index = hit.instance_index?;
        match hit.name.as_str() {
            "points" => {
                let [x, y] = self.points[index];
                Some(format!("Point {index}: ({x:.2}, {y:.2})"))
            }
            "bars" => Some(format!("Bin {index}: {} points", self.bins[index])),
            _ => None,
        }
    }
}

// egui shows textures in Rgba8Unorm, which needs no sRGB conversion.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A texture that holds two charts of `size` side by side.
fn chart_texture(device: &wgpu::Device, size: [f32; 2], scale: f32) -> OffscreenTarget {
    OffscreenTarget::new(
        device,
        &OffscreenTargetDescriptor::new(
            CanvasDimensions {
                size: [2.0 * size[0], size[1]],
                scale,
            },
            FORMAT,
        ),
    )
}

/// The plot area of a chart that is `width` wide: left, top, width, height.
fn plot_area(width: f32) -> [f32; 4] {
    [36.0, 36.0, width - 52.0, CHART_HEIGHT - 54.0]
}

/// `count` points in the unit square, from a fixed seed.
fn points(count: usize) -> Vec<[f32; 2]> {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 40) as f32 / (1u64 << 24) as f32
    };
    (0..count)
        .map(|_| {
            let x = next();
            // Points crowd toward the diagonal.
            [x, (x + 0.35 * (next() - 0.5)).clamp(0.0, 1.0)]
        })
        .collect()
}

/// How many points fall in each of `BINS` equal ranges of x.
fn bins(points: &[[f32; 2]]) -> Vec<usize> {
    let mut bins = vec![0; BINS];
    for [x, _] in points {
        bins[((x * BINS as f32) as usize).min(BINS - 1)] += 1;
    }
    bins
}

fn scatter(points: &[[f32; 2]], settings: Settings, theme: &Theme, chart_width: f32) -> SceneGraph {
    let [left, top, width, height] = plot_area(chart_width);
    let symbols = SceneSymbolMark {
        name: "points".to_string(),
        len: points.len() as u32,
        x: points
            .iter()
            .map(|[x, _]| left + x * width)
            .collect::<Vec<_>>()
            .into(),
        y: points
            .iter()
            .map(|[_, y]| top + (1.0 - y) * height)
            .collect::<Vec<_>>()
            .into(),
        size: settings.size.into(),
        fill: ColorOrGradient::Color(with_alpha(theme.points, settings.opacity)).into(),
        ..Default::default()
    };
    chart("Scatter", symbols.into(), theme, chart_width)
}

fn histogram(bins: &[usize], settings: Settings, theme: &Theme, chart_width: f32) -> SceneGraph {
    let [left, top, width, height] = plot_area(chart_width);
    let most = bins.iter().copied().max().unwrap_or(1).max(1) as f32;
    let bar_width = width / bins.len() as f32;
    let bars = SceneRectMark {
        name: "bars".to_string(),
        len: bins.len() as u32,
        x: (0..bins.len())
            .map(|i| left + i as f32 * bar_width + 2.0)
            .collect::<Vec<_>>()
            .into(),
        y: bins
            .iter()
            .map(|count| top + height * (1.0 - *count as f32 / most))
            .collect::<Vec<_>>()
            .into(),
        width: Some((bar_width - 4.0).into()),
        height: Some(
            bins.iter()
                .map(|count| height * *count as f32 / most)
                .collect::<Vec<_>>()
                .into(),
        ),
        fill: ColorOrGradient::Color(with_alpha(theme.bars, settings.opacity)).into(),
        ..Default::default()
    };
    chart("Histogram of x", bars.into(), theme, chart_width)
}

fn with_alpha([r, g, b]: [f32; 3], alpha: f32) -> [f32; 4] {
    [r, g, b, alpha]
}

/// A chart: a translucent plot area that lets egui's panel show through, the marks, and a title.
fn chart(
    title: &str,
    marks: avenger_scenegraph::marks::mark::SceneMark,
    theme: &Theme,
    chart_width: f32,
) -> SceneGraph {
    let [left, top, width, height] = plot_area(chart_width);
    let plot_area = SceneRectMark {
        x: left.into(),
        y: top.into(),
        width: Some(width.into()),
        height: Some(height.into()),
        fill: ColorOrGradient::Color(theme.plot).into(),
        stroke: ColorOrGradient::Color(theme.border).into(),
        stroke_width: 1.0.into(),
        interactive: Some(false),
        ..Default::default()
    };
    let title = SceneTextMark {
        text: title.to_string().into(),
        x: left.into(),
        y: (top - 10.0).into(),
        font_size: 15.0.into(),
        color: theme.text.into(),
        interactive: Some(false),
        ..Default::default()
    };
    SceneGraph {
        width: chart_width,
        height: CHART_HEIGHT,
        origin: [0.0, 0.0],
        marks: vec![plot_area.into(), marks, title.into()],
    }
}
