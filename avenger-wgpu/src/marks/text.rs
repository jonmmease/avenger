use crate::canvas::CanvasDimensions;
use avenger::marks::group::GroupBounds;
use avenger::marks::text::{
    FontStyleSpec, FontWeightNameSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec, TextMark,
};

use glyphon::{
    Attrs, Buffer, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Weight,
};
use itertools::izip;
use std::collections::HashSet;
use std::sync::Mutex;
use wgpu::{
    CommandBuffer, CommandEncoderDescriptor, Device, MultisampleState, Operations, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, TextureFormat, TextureView,
};

lazy_static! {
    static ref FONT_SYSTEM: Mutex<FontSystem> = Mutex::new(build_font_system());
    static ref SWASH_CACHE: Mutex<SwashCache> = Mutex::new(SwashCache::new());
}

fn build_font_system() -> FontSystem {
    let mut font_system = FontSystem::new();

    // Override default families based on what system fonts are available
    let fontdb = font_system.db_mut();
    let families: HashSet<String> = fontdb
        .faces()
        .flat_map(|face| {
            face.families
                .iter()
                .map(|(fam, _lang)| fam.clone())
                .collect::<Vec<_>>()
        })
        .collect();

    // Set default sans serif
    for family in ["Helvetica", "Arial", "Liberation Sans"] {
        if families.contains(family) {
            fontdb.set_sans_serif_family(family);
            break;
        }
    }

    // Set default monospace font family
    for family in [
        "Courier New",
        "Courier",
        "Liberation Mono",
        "DejaVu Sans Mono",
    ] {
        if families.contains(family) {
            fontdb.set_monospace_family(family);
            break;
        }
    }

    // Set default serif font family
    for family in [
        "Times New Roman",
        "Times",
        "Liberation Serif",
        "DejaVu Serif",
    ] {
        if families.contains(family) {
            fontdb.set_serif_family(family);
            break;
        }
    }

    font_system
}

#[derive(Clone, Debug)]
pub struct TextInstance {
    pub text: String,
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub align: TextAlignSpec,
    pub angle: f32,
    pub baseline: TextBaselineSpec,
    pub dx: f32,
    pub dy: f32,
    pub font: String,
    pub font_size: f32,
    pub font_weight: FontWeightSpec,
    pub font_style: FontStyleSpec,
    pub limit: f32,
}

impl TextInstance {
    pub fn iter_from_spec(
        mark: &TextMark,
        group_bounds: GroupBounds,
    ) -> impl Iterator<Item = TextInstance> + '_ {
        let group_x = group_bounds.x;
        let group_y = group_bounds.y;
        izip!(
            mark.text_iter(),
            mark.x_iter(),
            mark.y_iter(),
            mark.color_iter(),
            mark.align_iter(),
            mark.angle_iter(),
            mark.baseline_iter(),
            mark.dx_iter(),
            mark.dy_iter(),
            mark.font_iter(),
            mark.font_size_iter(),
            mark.font_weight_iter(),
            mark.font_style_iter(),
            mark.limit_iter(),
        )
        .map(
            move |(
                text,
                x,
                y,
                color,
                align,
                angle,
                baseline,
                dx,
                dy,
                font,
                font_size,
                font_weight,
                font_style,
                limit,
            )| {
                TextInstance {
                    text: text.clone(),
                    position: [*x + group_x, *y + group_y],
                    color: *color,
                    align: *align,
                    angle: *angle,
                    baseline: *baseline,
                    dx: *dx,
                    dy: *dy,
                    font: font.clone(),
                    font_size: *font_size,
                    font_weight: *font_weight,
                    font_style: *font_style,
                    limit: *limit,
                }
            },
        )
    }
}

pub struct TextMarkRenderer {
    pub atlas: TextAtlas,
    pub text_renderer: TextRenderer,
    pub instances: Vec<TextInstance>,
    pub dimensions: CanvasDimensions,
    pub group_bounds: GroupBounds,
}

impl TextMarkRenderer {
    pub fn new(
        device: &Device,
        queue: &Queue,
        texture_format: TextureFormat,
        dimensions: CanvasDimensions,
        sample_count: u32,
        mark: &TextMark,
        group_bounds: GroupBounds,
    ) -> Self {
        let instances = TextInstance::iter_from_spec(mark, group_bounds).collect::<Vec<_>>();
        let mut atlas = TextAtlas::with_color_mode(device, queue, texture_format, ColorMode::Web);
        let text_renderer = TextRenderer::new(
            &mut atlas,
            device,
            MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            None,
        );

        Self {
            atlas,
            text_renderer,
            dimensions,
            instances,
            group_bounds,
        }
    }

    pub fn render(
        &mut self,
        device: &Device,
        queue: &Queue,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
    ) -> CommandBuffer {
        let mut font_system = FONT_SYSTEM
            .lock()
            .expect("Failed to acquire lock on FONT_SYSTEM");
        let mut cache = SWASH_CACHE
            .lock()
            .expect("Failed to acquire lock on SWASH_CACHE");

        // Collect buffer into a vector first so that they live as long as the text areas
        // that reference them below
        let buffers = self
            .instances
            .iter()
            .map(|instance| {
                // Ad-hoc size adjustment for better match with resvg
                let font_size_scale = 0.99f32;
                let mut buffer = Buffer::new(
                    &mut font_system,
                    Metrics::new(
                        instance.font_size * self.dimensions.scale * font_size_scale,
                        instance.font_size * self.dimensions.scale * font_size_scale,
                    ),
                );
                let family = match instance.font.to_lowercase().as_str() {
                    "serif" => Family::Serif,
                    "sans serif" | "sans-serif" => Family::SansSerif,
                    "cursive" => Family::Cursive,
                    "fantasy" => Family::Fantasy,
                    "monospace" => Family::Monospace,
                    _ => Family::Name(instance.font.as_str()),
                };
                let weight = match instance.font_weight {
                    FontWeightSpec::Name(FontWeightNameSpec::Bold) => Weight::BOLD,
                    FontWeightSpec::Name(FontWeightNameSpec::Normal) => Weight::NORMAL,
                    FontWeightSpec::Number(w) => Weight(w as u16),
                };

                buffer.set_text(
                    &mut font_system,
                    &instance.text,
                    Attrs::new().family(family).weight(weight),
                    Shaping::Advanced,
                );
                buffer.set_size(
                    &mut font_system,
                    self.dimensions.size[0] * self.dimensions.scale,
                    self.dimensions.size[1] * self.dimensions.scale,
                );
                buffer.shape_until_scroll(&mut font_system);

                buffer
            })
            .collect::<Vec<_>>();

        let areas = buffers
            .iter()
            .zip(&self.instances)
            .map(|(buffer, instance)| {
                let (width, line_y, height) = measure(buffer);
                let scaled_x = instance.position[0] * self.dimensions.scale;
                let scaled_y = instance.position[1] * self.dimensions.scale;
                let left = match instance.align {
                    TextAlignSpec::Left => scaled_x,
                    TextAlignSpec::Center => scaled_x - width / 2.0,
                    TextAlignSpec::Right => scaled_x - width,
                };

                let mut top = match instance.baseline {
                    TextBaselineSpec::Alphabetic => scaled_y - line_y,
                    TextBaselineSpec::Top => scaled_y,
                    TextBaselineSpec::Middle => scaled_y - height * 0.5,
                    TextBaselineSpec::Bottom => scaled_y - height,
                    TextBaselineSpec::LineTop => todo!(),
                    TextBaselineSpec::LineBottom => todo!(),
                };

                // Add half pixel for top baseline for better match with resvg
                top += 0.5 * self.dimensions.scale;

                TextArea {
                    buffer,
                    left,
                    top,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: (self.dimensions.size[0] * self.dimensions.scale) as i32,
                        bottom: (self.dimensions.size[1] * self.dimensions.scale) as i32,
                    },
                    default_color: Color::rgba(
                        (instance.color[0] * 255.0) as u8,
                        (instance.color[1] * 255.0) as u8,
                        (instance.color[2] * 255.0) as u8,
                        (instance.color[3] * 255.0) as u8,
                    ),
                    angle: instance.angle,
                    rotation_origin: Some([scaled_x, scaled_y]),
                }
            })
            .collect::<Vec<_>>();

        self.text_renderer
            .prepare(
                device,
                queue,
                &mut font_system,
                &mut self.atlas,
                Resolution {
                    width: self.dimensions.to_physical_width(),
                    height: self.dimensions.to_physical_height(),
                },
                areas,
                &mut cache,
            )
            .unwrap();

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Text render"),
        });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: texture_view,
                    resolve_target,
                    ops: Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.text_renderer.render(&self.atlas, &mut pass).unwrap();
        }

        encoder.finish()
    }
}

pub fn measure(buffer: &Buffer) -> (f32, f32, f32) {
    let (width, line_y, total_lines) =
        buffer
            .layout_runs()
            .fold((0.0, 0.0, 0usize), |(width, line_y, total_lines), run| {
                (
                    run.line_w.max(width),
                    run.line_y.max(line_y),
                    total_lines + 1,
                )
            });
    (
        width,
        line_y,
        (total_lines as f32 * buffer.metrics().line_height),
    )
}
