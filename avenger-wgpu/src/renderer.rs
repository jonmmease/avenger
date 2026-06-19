use std::{collections::HashMap, sync::Arc};

use avenger_color::ColorOrGradient;
use avenger_common::{
    canvas::CanvasDimensions, types::LinearScaleAdjustment, value::ScalarOrArray,
};
use avenger_scenegraph::marks::{group::Clip, rect::SceneRectMark};
use wgpu::{
    BindGroup, CommandBuffer, CommandEncoderDescriptor, Device, Extent3d, Operations, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, StoreOp, TextureFormat, TextureView,
};

use crate::{
    canvas::{CanvasConfig, CanvasFrameOverlay, TextBuildCtor},
    error::AvengerWgpuError,
    marks::{
        instanced_mark::InstancedMarkRenderer,
        multi::{MultiMarkRenderResources, MultiMarkRenderer},
        text::TextAtlasBuilderTrait,
    },
    offscreen::{OffscreenTarget, RenderedOffscreenFrame},
    target::{AvengerRenderTarget, WHITE_CLEAR},
    zindex_layers::compute_zindex_layers,
};

#[derive(Clone)]
pub enum MarkRenderer {
    Instanced {
        renderer: Arc<InstancedMarkRenderer>,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    },
    /// A contiguous run of multi-marks at one z-index, stored as a half-open range
    /// of batch indices into the canvas's single shared `MultiMarkRenderer`. The
    /// renderer is prepared once per frame; consecutive runs are coalesced and
    /// encoded via `encode_multi_ranges`, preserving exact draw order and clipping.
    Multi { batch_range: std::ops::Range<usize> },
}

/// A mark renderer with its associated z-index.
#[derive(Clone)]
pub struct ZIndexedMark {
    pub zindex: i32,
    pub renderer: MarkRenderer,
}

pub(crate) fn mark_renderer_counts(marks: &[ZIndexedMark]) -> (usize, usize) {
    marks
        .iter()
        .fold((0, 0), |(instanced_count, multi_count), mark| {
            match &mark.renderer {
                MarkRenderer::Instanced { .. } => (instanced_count + 1, multi_count),
                MarkRenderer::Multi { .. } => (instanced_count, multi_count + 1),
            }
        })
}

/// Surface-independent renderer state shared by host canvases.
pub(crate) struct AvengerRendererCore {
    dimensions: CanvasDimensions,
    texture_format: TextureFormat,
    sample_count: u32,
    marks: Vec<ZIndexedMark>,
    // All multi-marks accumulate into one shared renderer; each z-run is recorded
    // in `marks` as a batch range. `run_start` is the batch index where the current
    // (uncommitted) z-run began.
    shared_multi: MultiMarkRenderer,
    run_start: usize,
    current_zindex: i32,
    instanced_renderers: HashMap<u64, Arc<InstancedMarkRenderer>>,
    multi_render_resources: MultiMarkRenderResources,
    config: CanvasConfig,
    // Text atlas shared by all multi-renderers; built + uploaded once per frame.
    text_atlas_builder: Box<dyn TextAtlasBuilderTrait>,
}

impl AvengerRendererCore {
    pub(crate) fn new(
        device: &Device,
        dimensions: CanvasDimensions,
        texture_format: TextureFormat,
        sample_count: u32,
        config: CanvasConfig,
    ) -> Self {
        let multi_render_resources =
            MultiMarkRenderResources::new(device, texture_format, sample_count);
        let text_atlas_builder = make_text_atlas_builder(&config.text_builder_ctor);

        Self {
            dimensions,
            texture_format,
            sample_count,
            marks: Vec::new(),
            shared_multi: MultiMarkRenderer::new(dimensions),
            run_start: 0,
            current_zindex: 0,
            instanced_renderers: HashMap::new(),
            multi_render_resources,
            config,
            text_atlas_builder,
        }
    }

    pub(crate) fn dimensions(&self) -> CanvasDimensions {
        self.dimensions
    }

    pub(crate) fn set_dimensions(&mut self, dimensions: CanvasDimensions) {
        self.dimensions = dimensions;
    }

    pub(crate) fn texture_format(&self) -> TextureFormat {
        self.texture_format
    }

    pub(crate) fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub(crate) fn marks(&self) -> &[ZIndexedMark] {
        &self.marks
    }

    pub(crate) fn shared_multi_mut(&mut self) -> &mut MultiMarkRenderer {
        &mut self.shared_multi
    }

    pub(crate) fn text_atlas_builder_mut(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        &mut *self.text_atlas_builder
    }

    pub(crate) fn build_text_bind_groups(&self, device: &Device, queue: &Queue) -> Vec<BindGroup> {
        let (text_atlas_size, text_atlas_images) = self.text_atlas_builder.build();
        MultiMarkRenderer::make_text_bind_groups_dual_sampler(
            device,
            queue,
            self.multi_render_resources.text_layout(),
            text_atlas_size,
            &text_atlas_images,
        )
    }

    pub(crate) fn make_background_command(
        &self,
        device: &Device,
        target: AvengerRenderTarget<'_>,
    ) -> CommandBuffer {
        debug_assert_eq!(target.format, self.texture_format);
        Self::make_background_command_for_target(device, target)
    }

    pub(crate) fn make_background_command_for_target(
        device: &Device,
        target: AvengerRenderTarget<'_>,
    ) -> CommandBuffer {
        let mut background_encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Render Background Encoder"),
        });

        {
            let _render_pass = background_encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target.view,
                    resolve_target: target.resolve_target,
                    depth_slice: None,
                    ops: Operations {
                        load: target.load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }
        background_encoder.finish()
    }

    #[allow(dead_code)]
    pub(crate) fn build_frame_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: AvengerRenderTarget<'_>,
        overlay: Option<CanvasFrameOverlay>,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        debug_assert_eq!(target.format, self.texture_format);
        debug_assert_eq!(target.sample_count, self.sample_count);

        self.commit_all_multi_renderers();
        let marks = self.marks.clone();
        let zindices: Vec<i32> = marks.iter().map(|m| m.zindex).collect();
        let layers = if zindices.is_empty() {
            vec![]
        } else {
            compute_zindex_layers(zindices)
        };

        let background_command = self.make_background_command(device, target);
        let mut commands = vec![background_command];

        let multi_render_resources = self.multi_render_resources.clone();
        let text_bind_groups = self.build_text_bind_groups(device, queue);
        let prepared =
            self.shared_multi
                .prepare(device, queue, target.extent, &multi_render_resources);

        let mut mark_encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Avenger Mark Render Encoder"),
        });
        let mut encoded_marks = false;
        let mut pending: Vec<std::ops::Range<usize>> = Vec::new();
        for (min_z, max_z) in layers {
            for mark in &marks {
                if mark.zindex >= min_z && mark.zindex <= max_z {
                    match &mark.renderer {
                        MarkRenderer::Multi { batch_range } => pending.push(batch_range.clone()),
                        MarkRenderer::Instanced {
                            renderer,
                            x_adjustment,
                            y_adjustment,
                        } => {
                            if !pending.is_empty() {
                                self.shared_multi.encode_multi_ranges_into(
                                    &mut mark_encoder,
                                    target.extent,
                                    target.view,
                                    target.resolve_target,
                                    &multi_render_resources,
                                    &text_bind_groups,
                                    &prepared,
                                    &pending,
                                );
                                pending.clear();
                            }
                            renderer.encode_into(
                                device,
                                &mut mark_encoder,
                                target.view,
                                target.resolve_target,
                                *x_adjustment,
                                *y_adjustment,
                            );
                            encoded_marks = true;
                        }
                    }
                }
            }
        }

        if !pending.is_empty() {
            self.shared_multi.encode_multi_ranges_into(
                &mut mark_encoder,
                target.extent,
                target.view,
                target.resolve_target,
                &multi_render_resources,
                &text_bind_groups,
                &prepared,
                &pending,
            );
            encoded_marks = true;
        }

        if encoded_marks {
            commands.push(mark_encoder.finish());
        }

        if let Some(command) = self.make_frame_overlay_command(
            device,
            queue,
            overlay,
            target.extent,
            target.view,
            target.resolve_target,
            &text_bind_groups,
        )? {
            commands.push(command);
        }

        Ok(commands)
    }

    #[allow(dead_code)]
    pub(crate) fn encode_to_offscreen_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &mut OffscreenTarget,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        let render_target = target.render_target(WHITE_CLEAR);
        self.build_frame_commands(device, queue, render_target, None)
    }

    #[allow(dead_code)]
    pub(crate) fn render_to_offscreen(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &mut OffscreenTarget,
    ) -> Result<RenderedOffscreenFrame, AvengerWgpuError> {
        let commands = self.encode_to_offscreen_commands(device, queue, target)?;
        queue.submit(commands);
        Ok(RenderedOffscreenFrame::from(&*target))
    }

    pub(crate) fn set_current_zindex(&mut self, zindex: i32) {
        if zindex != self.current_zindex {
            self.commit_multi_renderer_if_needed();
        }
        self.current_zindex = zindex;
    }

    pub(crate) fn commit_multi_renderer_if_needed(&mut self) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(ZIndexedMark {
                zindex: self.current_zindex,
                renderer: MarkRenderer::Multi {
                    batch_range: self.run_start..end,
                },
            });
        }
        self.run_start = end;
    }

    pub(crate) fn commit_all_multi_renderers(&mut self) {
        self.commit_multi_renderer_if_needed();
    }

    pub(crate) fn current_zindex(&self) -> i32 {
        self.current_zindex
    }

    pub(crate) fn get_instanced_renderer(
        &self,
        fingerprint: u64,
    ) -> Option<Arc<InstancedMarkRenderer>> {
        self.instanced_renderers.get(&fingerprint).cloned()
    }

    pub(crate) fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        self.commit_multi_renderer_if_needed();
        self.instanced_renderers
            .insert(fingerprint, mark_renderer.clone());
        self.marks.push(ZIndexedMark {
            zindex: self.current_zindex,
            renderer: MarkRenderer::Instanced {
                renderer: mark_renderer,
                x_adjustment,
                y_adjustment,
            },
        });
    }

    pub(crate) fn clear_mark_renderer(&mut self) {
        // One shared multi-renderer per canvas: reset it in place each frame and
        // clear the recorded z-run marks. (Instanced fingerprint cache is retained.)
        self.shared_multi.reset_for_frame(self.dimensions);
        self.run_start = 0;
        self.marks.clear();

        // Reset the shared text atlas so each frame starts clean (matches the old
        // per-renderer reset-per-frame semantics). `TextAtlasBuilder` has no reset
        // method, so replace it with a fresh builder via the same ctor.
        self.text_atlas_builder = make_text_atlas_builder(&self.config.text_builder_ctor);
    }

    pub(crate) fn make_frame_overlay_command(
        &self,
        device: &Device,
        queue: &Queue,
        overlay: Option<CanvasFrameOverlay>,
        render_target_extent: Extent3d,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
        text_bind_groups: &[BindGroup],
    ) -> Result<Option<CommandBuffer>, AvengerWgpuError> {
        let Some(overlay) = overlay else {
            return Ok(None);
        };

        let width = overlay.size[0].max(0.0);
        let height = overlay.size[1].max(0.0);
        if width <= 0.0 || height <= 0.0 {
            return Ok(None);
        }

        let border = 1.0;
        let handle = overlay.handle_thickness.max(border);
        let outline = ColorOrGradient::Color([0.12, 0.15, 0.18, 0.95]);
        let handle_fill = ColorOrGradient::Color([0.12, 0.15, 0.18, 0.16]);

        let mut x = vec![0.0, 0.0, 0.0, (width - border).max(0.0)];
        let mut y = vec![0.0, (height - border).max(0.0), 0.0, 0.0];
        let mut rect_width = vec![width, width, border, border];
        let mut rect_height = vec![border, border, height, height];
        let mut fill = vec![
            outline.clone(),
            outline.clone(),
            outline.clone(),
            outline.clone(),
        ];

        if overlay.resize_width {
            x.push((width - handle).max(0.0));
            y.push(0.0);
            rect_width.push(handle);
            rect_height.push(height);
            fill.push(handle_fill.clone());
        }

        if overlay.resize_height {
            x.push(0.0);
            y.push((height - handle).max(0.0));
            rect_width.push(width);
            rect_height.push(handle);
            fill.push(handle_fill);
        }

        let mark = SceneRectMark {
            name: "canvas_frame_overlay".to_string(),
            clip: false,
            len: x.len() as u32,
            gradients: Vec::new(),
            x: ScalarOrArray::new_array(x),
            y: ScalarOrArray::new_array(y),
            width: Some(ScalarOrArray::new_array(rect_width)),
            height: Some(ScalarOrArray::new_array(rect_height)),
            x2: None,
            y2: None,
            fill: ScalarOrArray::new_array(fill),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::transparent()),
            stroke_width: ScalarOrArray::new_scalar(0.0),
            corner_radius: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: None,
            interactive: true,
        };

        let mut renderer = MultiMarkRenderer::new(self.dimensions);
        renderer.add_rect_mark(&mark, [0.0, 0.0], &Clip::None)?;

        // The overlay has no text, but `render_with_resources` indexes
        // `text_bind_groups[0]`; reuse the frame's shared text bind groups.
        Ok(Some(renderer.render_with_resources(
            device,
            queue,
            render_target_extent,
            texture_view,
            resolve_target,
            &self.multi_render_resources,
            text_bind_groups,
        )))
    }
}

/// Construct a text atlas builder, shared by a single canvas across all of its
/// multi-renderers. This mirrors the construction logic that previously lived in
/// `MultiMarkRenderer::new`: honor a caller-supplied `text_builder_ctor`, otherwise
/// fall back to the cosmic-text rasterizer (native), the html-canvas rasterizer
/// (wasm), or the null builder (text disabled).
pub(crate) fn make_text_atlas_builder(
    text_builder_ctor: &Option<TextBuildCtor>,
) -> Box<dyn TextAtlasBuilderTrait> {
    if let Some(text_builder_ctor) = text_builder_ctor {
        text_builder_ctor()
    } else {
        cfg_if::cfg_if! {
            if #[cfg(feature = "cosmic-text")] {
                use crate::marks::text::TextAtlasBuilder;
                use std::sync::Arc;
                let inner_text_atlas_builder: Box<dyn TextAtlasBuilderTrait> = Box::new(TextAtlasBuilder::new(Arc::new(
                    avenger_text::rasterization::cosmic::CosmicTextRasterizer::<crate::marks::text::GlyphBBoxAndAtlasCoords>::new())
                ));
            } else if #[cfg(target_arch = "wasm32")] {
                use crate::marks::text::TextAtlasBuilder;
                use std::sync::Arc;
                let inner_text_atlas_builder: Box<dyn TextAtlasBuilderTrait> = Box::new(TextAtlasBuilder::new(Arc::new(
                    avenger_text::rasterization::html_canvas::HtmlCanvasTextRasterizer::<crate::marks::text::GlyphBBoxAndAtlasCoords>::new())
                ));
            } else {
                use crate::marks::text::NullTextAtlasBuilder;
                let inner_text_atlas_builder: Box<dyn TextAtlasBuilderTrait> = Box::new(NullTextAtlasBuilder);
            }
        };
        inner_text_atlas_builder
    }
}
