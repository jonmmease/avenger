use crate::renderers::canvas::CanvasUniform;
use crate::renderers::mark::MarkShader;
use crate::scene::text::TextInstance;
use crate::specs::text::{TextAlignSpec, TextBaselineSpec};
use glyphon::cosmic_text::Align;
use glyphon::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer,
};
use wgpu::{
    CommandBuffer, CommandEncoderDescriptor, Device, MultisampleState, Operations, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, TextureFormat, TextureView,
};

pub struct TextMarkRenderer {
    pub font_system: FontSystem,
    pub cache: SwashCache,
    pub atlas: TextAtlas,
    pub text_renderer: TextRenderer,
    pub instances: Vec<TextInstance>,
    pub uniform: CanvasUniform,
}

impl TextMarkRenderer {
    pub fn new(
        device: &Device,
        queue: &Queue,
        uniform: CanvasUniform,
        texture_format: TextureFormat,
        sample_count: u32,
        instances: Vec<TextInstance>,
    ) -> Self {
        let mut font_system = FontSystem::new();
        let mut cache = SwashCache::new();
        let mut atlas = TextAtlas::new(device, queue, texture_format);
        let mut text_renderer = TextRenderer::new(
            &mut atlas,
            &device,
            MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            None,
        );

        Self {
            font_system,
            cache,
            atlas,
            text_renderer,
            uniform,
            instances,
        }
    }

    pub fn render(
        &mut self,
        device: &Device,
        queue: &Queue,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
    ) -> CommandBuffer {
        // Collect buffer into a vector first so that they live as long as the text areas
        // that reference them below
        let buffers = self
            .instances
            .iter()
            .map(|instance| {
                let mut buffer = Buffer::new(
                    &mut self.font_system,
                    Metrics::new(instance.font_size, instance.font_size * 1.0),
                );
                buffer.set_text(
                    &mut self.font_system,
                    &instance.text,
                    Attrs::new().family(Family::SansSerif),
                    Shaping::Advanced,
                );
                buffer.set_size(
                    &mut self.font_system,
                    self.uniform.size[0],
                    self.uniform.size[1],
                );
                buffer.shape_until_scroll(&mut self.font_system);

                buffer
            })
            .collect::<Vec<_>>();

        let areas = buffers
            .iter()
            .zip(&self.instances)
            .map(|(buffer, instance)| {
                let (width, height) = measure(&buffer);

                let left = match instance.align {
                    TextAlignSpec::Left => instance.position[0],
                    TextAlignSpec::Center => instance.position[0] - width / 2.0,
                    TextAlignSpec::Right => instance.position[0] - width,
                };

                let top = match instance.baseline {
                    TextBaselineSpec::Alphabetic => instance.position[1] - height,
                    TextBaselineSpec::Top => instance.position[1],
                    TextBaselineSpec::Middle => instance.position[1] - height * 0.56,
                    TextBaselineSpec::Bottom => instance.position[1] - height,
                    TextBaselineSpec::LineTop => todo!(),
                    TextBaselineSpec::LineBottom => todo!(),
                };

                TextArea {
                    buffer: &buffer,
                    left,
                    top,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: self.uniform.size[0] as i32,
                        bottom: self.uniform.size[1] as i32,
                    },
                    default_color: Color::rgb(
                        (instance.color[0] * 255.0) as u8,
                        (instance.color[1] * 255.0) as u8,
                        (instance.color[2] * 255.0) as u8,
                    ),
                }
            })
            .collect::<Vec<_>>();

        self.text_renderer
            .prepare(
                &device,
                &queue,
                &mut self.font_system,
                &mut self.atlas,
                Resolution {
                    width: self.uniform.size[0] as u32,
                    height: self.uniform.size[1] as u32,
                },
                areas,
                &mut self.cache,
            )
            .unwrap();

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Text render"),
        });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &texture_view,
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

pub fn measure(buffer: &Buffer) -> (f32, f32) {
    let (width, total_lines) = buffer
        .layout_runs()
        .fold((0.0, 0usize), |(width, total_lines), run| {
            (run.line_w.max(width), total_lines + 1)
        });
    (width, (total_lines as f32 * buffer.metrics().line_height))
}
