use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use avenger_common::{canvas::CanvasDimensions, time::Instant, types::LinearScaleAdjustment};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark,
        area::SceneAreaMark,
        group::Clip,
        group::SceneGroup,
        image::SceneImageMark,
        line::SceneLineMark,
        mark::SceneMark,
        path::ScenePathMark,
        pattern::{is_no_fill_pattern, PatternReferenceFrame},
        rect::SceneRectMark,
        rule::SceneRuleMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
        text_leader::{compute_text_leader_geometry, TextLeaderGeometryInput},
        trail::SceneTrailMark,
    },
    pattern_geometry::PatternRect,
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use avenger_text::{measurement::TextMeasurementConfig, FontResolutionOptions, TextEngine};
use itertools::izip;
use wgpu::{
    Adapter, CommandEncoderDescriptor, Device, DeviceDescriptor, Extent3d, PowerPreference, Queue,
    RequestAdapterError, RequestAdapterOptions, Surface, SurfaceConfiguration, TextureDescriptor,
    TextureDimension, TextureFormat, TextureFormatFeatureFlags, TextureUsages, TextureView,
    TextureViewDescriptor, Trace,
};
use winit::{
    dpi::{PhysicalSize, Size},
    event::WindowEvent,
    window::Window,
};

use crate::{
    error::AvengerWgpuError,
    marks::{
        instanced_mark::{InstancedMarkFingerprint, InstancedMarkRenderer},
        multi::{is_axis_aligned_angle, MultiMarkRenderer, TextLeaderRenderItem},
        symbol::{is_circle_only_symbol_mark, CircleSymbolShader, SymbolShader},
        text::{TextAtlasBuilderTrait, TextAtlasRegistration, TextInstance},
    },
    offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
    readback::TextureReadback,
    renderer::{mark_renderer_counts, AvengerRendererCore},
    target::{AvengerRenderTarget, WHITE_CLEAR},
};
use avenger_scenegraph::render_order::compute_zindex_layers;

pub use crate::renderer::{MarkRenderer, ZIndexedMark};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasFrameOverlay {
    pub size: [f32; 2],
    pub resize_width: bool,
    pub resize_height: bool,
    pub handle_thickness: f32,
}

pub type TextBuildCtor = Arc<fn() -> Box<dyn TextAtlasBuilderTrait>>;

pub trait CanvasDimensionUtils {
    fn to_physical_size(&self) -> winit::dpi::PhysicalSize<u32>;
}

fn instanced_symbol_renderer_cache_key(
    mark: &SceneSymbolMark,
    origin: [f32; 2],
    dimensions: CanvasDimensions,
    clip: &Clip,
) -> u64 {
    let mut hasher = DefaultHasher::new();

    // The mark fingerprint covers the geometry and per-instance data. The remaining
    // values are baked into immutable renderer uniforms and therefore must also match
    // before an existing renderer can be reused.
    "symbol".hash(&mut hasher);
    mark.instanced_fingerprint().hash(&mut hasher);
    origin.map(f32::to_bits).hash(&mut hasher);
    dimensions.size.map(f32::to_bits).hash(&mut hasher);
    dimensions.scale.to_bits().hash(&mut hasher);
    clip.hash(&mut hasher);

    hasher.finish()
}

impl CanvasDimensionUtils for CanvasDimensions {
    fn to_physical_size(&self) -> winit::dpi::PhysicalSize<u32> {
        winit::dpi::PhysicalSize {
            width: self.to_physical_width(),
            height: self.to_physical_height(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn window_accepted_or_requested_size(
    requested_size: PhysicalSize<u32>,
    _accepted_size: PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    requested_size
}

#[cfg(not(target_arch = "wasm32"))]
fn window_accepted_or_requested_size(
    _requested_size: PhysicalSize<u32>,
    accepted_size: PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    accepted_size
}

#[derive(Clone)]
pub struct CanvasConfig {
    pub text_builder_ctor: Option<TextBuildCtor>,
    pub font_resolution: FontResolutionOptions,
    /// Shared layout and raster context. When supplied, takes precedence over
    /// font_resolution. Pass clones to guides and interaction geometry too.
    pub text_engine: Option<TextEngine>,
    pub sample_count: Option<u32>,
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            text_builder_ctor: None,
            text_engine: None,
            font_resolution: avenger_text::default_font_resolution(),
            sample_count: None,
        }
    }
}

impl CanvasConfig {
    /// Resolve once and share the returned engine with every scene consumer.
    pub fn resolved_text_engine(&self) -> TextEngine {
        self.text_engine.clone().unwrap_or_else(|| {
            TextEngine::with_font_resolution(&self.font_resolution)
                .expect("failed to initialize text engine")
        })
    }
}

pub trait Canvas {
    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    );
    fn clear_mark_renderer(&mut self);

    /// Start installing a scene, retaining its resources for subsequent frames.
    fn begin_scene(&mut self, _scene: &SceneGraph) {
        self.clear_mark_renderer();
    }

    /// Mark a scene installation as complete.
    fn finish_scene(&mut self) {}
    fn device(&self) -> &Device;
    fn queue(&self) -> &Queue;
    fn dimensions(&self) -> CanvasDimensions;

    fn font_resolution(&self) -> &FontResolutionOptions;

    fn text_engine(&self) -> TextEngine {
        TextEngine::with_font_resolution(self.font_resolution())
            .expect("failed to initialize text engine")
    }

    fn texture_format(&self) -> TextureFormat;

    fn sample_count(&self) -> u32;

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer;

    /// The text atlas shared by every multi-renderer on this canvas. Glyphs are
    /// registered into it during the set_scene mark walk, and it is built + uploaded
    /// once per frame at render time.
    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait;

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>>;

    fn set_current_zindex(&mut self, zindex: i32);

    fn commit_multi_renderer_if_needed(&mut self, new_zindex: i32);

    fn get_current_zindex(&self) -> i32;

    fn add_arc_mark(
        &mut self,
        mark: &SceneArcMark,
        origin: [f32; 2],
        group_clip: &Clip,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_arc_mark(
            mark,
            origin,
            group_clip,
            pattern_reference_frame,
            chart_bounds,
        )?;
        Ok(())
    }

    fn add_path_mark(
        &mut self,
        mark: &ScenePathMark,
        origin: [f32; 2],
        group_clip: &Clip,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_path_mark(
            mark,
            origin,
            group_clip,
            pattern_reference_frame,
            chart_bounds,
        )?;
        Ok(())
    }

    fn add_line_mark(
        &mut self,
        mark: &SceneLineMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_line_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_trail_mark(
        &mut self,
        mark: &SceneTrailMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_trail_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_area_mark(
        &mut self,
        mark: &SceneAreaMark,
        origin: [f32; 2],
        group_clip: &Clip,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_area_mark(
            mark,
            origin,
            group_clip,
            pattern_reference_frame,
            chart_bounds,
        )?;
        Ok(())
    }

    fn add_symbol_mark(
        &mut self,
        mark: &SceneSymbolMark,
        origin: [f32; 2],
        group_clip: &Clip,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        if symbol_mark_is_instanced_eligible(mark, group_clip) {
            let effective_clip = group_clip.maybe_clip(mark.clip);
            let fingerprint = instanced_symbol_renderer_cache_key(
                mark,
                origin,
                self.dimensions(),
                &effective_clip,
            );
            let renderer = if let Some(renderer) = self.get_instanced_renderer(fingerprint) {
                renderer
            } else if is_circle_only_symbol_mark(mark) {
                let shader = Box::new(CircleSymbolShader::from_symbol_mark(
                    mark,
                    self.dimensions(),
                    origin,
                ));

                let renderer = Arc::new(InstancedMarkRenderer::new(
                    self.device(),
                    self.texture_format(),
                    self.sample_count(),
                    shader,
                    effective_clip,
                    self.dimensions().scale,
                ));
                renderer
            } else {
                let shader = Box::new(SymbolShader::from_symbol_mark(
                    mark,
                    self.dimensions(),
                    origin,
                )?);

                let renderer = Arc::new(InstancedMarkRenderer::new(
                    self.device(),
                    self.texture_format(),
                    self.sample_count(),
                    shader,
                    effective_clip,
                    self.dimensions().scale,
                ));
                renderer
            };

            self.add_instanced_mark_renderer(
                renderer,
                fingerprint,
                mark.x_adjustment,
                mark.y_adjustment,
            );
        } else {
            self.get_multi_renderer().add_symbol_mark(
                mark,
                origin,
                group_clip,
                pattern_reference_frame,
                chart_bounds,
            )?;
        }

        Ok(())
    }

    fn add_rect_mark(
        &mut self,
        mark: &SceneRectMark,
        origin: [f32; 2],
        group_clip: &Clip,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer().add_rect_mark(
            mark,
            origin,
            group_clip,
            pattern_reference_frame,
            chart_bounds,
        )?;
        Ok(())
    }

    fn add_rule_mark(
        &mut self,
        mark: &SceneRuleMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_rule_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_text_mark(
        &mut self,
        mark: &SceneTextMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        // The atlas and leader geometry share the same text context.
        let dimensions = self.dimensions();
        let text_engine = self.text_engine();
        let leader_stroke_dash_values = mark
            .leader_stroke_dash
            .as_ref()
            .map(|dash| dash.as_vec(mark.len as usize, mark.indices.as_ref()));
        let text_atlas_builder = self.text_atlas_builder();
        let mut leaders = Vec::new();
        let registrations: Vec<TextAtlasRegistration> = izip!(
            mark.text_iter(),
            mark.target_position_iter(),
            mark.label_position_iter(),
            mark.defined_iter(),
            mark.color_iter(),
            mark.align_iter(),
            mark.angle_iter(),
            mark.baseline_iter(),
            mark.font_iter(),
            mark.font_size_iter(),
            mark.font_weight_iter(),
            mark.font_style_iter(),
            mark.limit_iter(),
            mark.leader_iter(),
            mark.leader_stroke_iter(),
            mark.leader_stroke_width_iter(),
            mark.leader_stroke_cap_iter(),
            mark.leader_stroke_join_iter(),
            mark.leader_label_padding_iter(),
            mark.leader_target_radius_iter(),
            mark.leader_min_length_iter(),
            mark.leader_shape_iter(),
            mark.leader_arrow_iter(),
            mark.leader_arrow_length_iter(),
            mark.leader_arrow_width_iter(),
        )
        .enumerate()
        .map(
            |(
                index,
                (
                    text,
                    target,
                    label,
                    defined,
                    color,
                    align,
                    angle,
                    baseline,
                    font,
                    font_size,
                    font_weight,
                    font_style,
                    limit,
                    leader,
                    leader_stroke,
                    leader_stroke_width,
                    leader_stroke_cap,
                    leader_stroke_join,
                    leader_label_padding,
                    leader_target_radius,
                    leader_min_length,
                    leader_shape,
                    leader_arrow,
                    leader_arrow_length,
                    leader_arrow_width,
                ),
            )| {
                if !*defined {
                    return Ok(Vec::new());
                }

                let use_nearest_filter = is_axis_aligned_angle(*angle);
                let label = [label[0] + origin[0], label[1] + origin[1]];
                let instance = TextInstance {
                    text,
                    position: label,
                    color: &color.color_or_transparent(),
                    align,
                    angle: *angle,
                    baseline,
                    font,
                    font_size: *font_size,
                    font_weight,
                    font_style,
                    syntax_mode: mark.text_syntax,
                    params: &mark.text_params,
                    number_locale: mark.number_locale.as_deref(),
                    number_locale_specs: &mark.number_locale_specs,
                    datetime_locale: mark.datetime_locale.as_deref(),
                    datetime_timezone: mark.datetime_timezone.as_deref(),
                    datetime_locale_specs: &mark.datetime_locale_specs,
                    limit: *limit,
                    use_nearest_filter,
                };
                if *leader {
                    let text_bounds = text_engine.measure_bounds_with_limit_or_approx(
                        &TextMeasurementConfig {
                            text,
                            font,
                            font_size: *font_size,
                            font_weight: *font_weight,
                            font_style: *font_style,
                            syntax_mode: mark.text_syntax,
                            params: &mark.text_params,
                            number_locale: mark.number_locale.as_deref(),
                            number_locale_specs: Some(&mark.number_locale_specs),
                            datetime_locale: mark.datetime_locale.as_deref(),
                            datetime_timezone: mark.datetime_timezone.as_deref(),
                            datetime_locale_specs: Some(&mark.datetime_locale_specs),
                        },
                        *limit,
                    );
                    if let Some(geometry) = compute_text_leader_geometry(TextLeaderGeometryInput {
                        target: [target[0] + origin[0], target[1] + origin[1]],
                        label_anchor: label,
                        angle_degrees: *angle,
                        text_bounds: &text_bounds,
                        align,
                        baseline,
                        label_padding: *leader_label_padding,
                        target_radius: *leader_target_radius,
                        min_length: *leader_min_length,
                        shape: *leader_shape,
                        arrow: *leader_arrow,
                        arrow_length: *leader_arrow_length,
                        arrow_width: *leader_arrow_width,
                    }) {
                        leaders.push(TextLeaderRenderItem {
                            geometry,
                            stroke: leader_stroke.clone(),
                            stroke_width: *leader_stroke_width,
                            stroke_cap: *leader_stroke_cap,
                            stroke_join: *leader_stroke_join,
                            stroke_dash: leader_stroke_dash_values
                                .as_ref()
                                .and_then(|values| values.get(index).cloned()),
                        });
                    }
                }
                text_atlas_builder.register_text(instance, dimensions)
            },
        )
        .collect::<Result<Vec<_>, AvengerWgpuError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        // The two borrows above (`text_atlas_builder`) and below (`get_multi_renderer`)
        // are sequential — `registrations` is owned — so there is no borrow conflict.
        self.get_multi_renderer()
            .add_text_leaders(leaders, group_clip, mark.clip)?;
        self.get_multi_renderer()
            .add_text_registrations(registrations, group_clip, mark.clip)?;
        Ok(())
    }

    fn add_image_mark(
        &mut self,
        mark: &SceneImageMark,
        origin: [f32; 2],
        group_clip: &Clip,
    ) -> Result<(), AvengerWgpuError> {
        self.get_multi_renderer()
            .add_image_mark(mark, origin, group_clip)?;
        Ok(())
    }

    fn add_group_mark(
        &mut self,
        group: &SceneGroup,
        parent_origin: [f32; 2],
        parent_clip: &Clip,
        parent_pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerWgpuError> {
        // Save parent z-index
        let saved_zindex = self.get_current_zindex();

        // Group's z-index defaults to parent's z-index
        let group_zindex = group.zindex.unwrap_or(saved_zindex);
        self.set_current_zindex(group_zindex);

        // Compute new origin
        let origin = [
            parent_origin[0] + group.origin[0],
            parent_origin[1] + group.origin[1],
        ];
        let pattern_reference_frame = group
            .pattern_reference_frame
            .as_ref()
            .map(|frame| frame.translated(origin[0], origin[1]))
            .or_else(|| parent_pattern_reference_frame.cloned());

        // Maybe add rect around group boundary
        if let Some(rect) = group.make_path_mark() {
            self.add_path_mark(
                &rect,
                parent_origin,
                &group.clip,
                pattern_reference_frame.as_ref(),
                chart_bounds,
            )?;
        }

        // Compute new clip
        let clip = if let Clip::None = group.clip {
            // No clip defined for this group, propagate parent clip down
            parent_clip.clone()
        } else {
            // Translate clip to absolute coordinates
            group.clip.translate(origin[0], origin[1])
        };

        for mark in &group.marks {
            // Mark inherits group's z-index if it doesn't have its own
            let mark_zindex = mark.zindex().unwrap_or(group_zindex);
            self.set_current_zindex(mark_zindex);

            match mark {
                SceneMark::Arc(mark) => {
                    self.add_arc_mark(
                        mark,
                        origin,
                        &clip,
                        pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
                SceneMark::Symbol(mark) => {
                    self.add_symbol_mark(
                        mark,
                        origin,
                        &clip,
                        pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
                SceneMark::Rect(mark) => {
                    self.add_rect_mark(
                        mark,
                        origin,
                        &clip,
                        pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
                SceneMark::Rule(mark) => {
                    self.add_rule_mark(mark, origin, &clip)?;
                }
                SceneMark::Path(mark) => {
                    self.add_path_mark(
                        mark,
                        origin,
                        &clip,
                        pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
                SceneMark::Line(mark) => {
                    self.add_line_mark(mark, origin, &clip)?;
                }
                SceneMark::Trail(mark) => {
                    self.add_trail_mark(mark, origin, &clip)?;
                }
                SceneMark::Area(mark) => {
                    self.add_area_mark(
                        mark,
                        origin,
                        &clip,
                        pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
                SceneMark::Text(mark) => {
                    self.add_text_mark(mark, origin, &clip)?;
                }
                SceneMark::Image(mark) => {
                    self.add_image_mark(mark, origin, &clip)?;
                }

                SceneMark::Group(group) => {
                    self.add_group_mark(
                        group,
                        origin,
                        &clip,
                        pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
            }
        }

        // Restore previous z-index
        self.set_current_zindex(saved_zindex);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn set_scene(&mut self, scene_graph: &SceneGraph) -> Result<(), AvengerWgpuError> {
        let start = Instant::now();
        let clear_start = Instant::now();
        self.begin_scene(scene_graph);
        self.set_current_zindex(0);
        let clear_elapsed = clear_start.elapsed();

        // Process display items in document order. Z-index sorting happens during rendering.
        let chart_bounds = PatternRect::new(0.0, 0.0, scene_graph.width, scene_graph.height);
        let display_list_start = Instant::now();
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);
        let display_list_elapsed = display_list_start.elapsed();
        let item_count = display_list.items.len();
        let mut image_ms = 0.0;
        let mut text_ms = 0.0;
        let mut other_ms = 0.0;
        let mut text_mark_count = 0u64;
        let mut text_instance_count = 0u64;
        let mut text_leader_count = 0u64;
        for item in &display_list.items {
            self.set_current_zindex(item.zindex);
            let item_start = Instant::now();
            let mut item_kind = "other";
            match &item.mark {
                SceneDisplayMark::OwnedGroupPath(mark) => {
                    self.add_path_mark(
                        mark,
                        item.origin,
                        &item.clip,
                        item.pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
                SceneDisplayMark::Borrowed(mark) => match mark {
                    SceneMark::Arc(mark) => {
                        self.add_arc_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Symbol(mark) => {
                        self.add_symbol_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Rect(mark) => {
                        self.add_rect_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Rule(mark) => {
                        self.add_rule_mark(mark, item.origin, &item.clip)?;
                    }
                    SceneMark::Path(mark) => {
                        self.add_path_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Line(mark) => {
                        self.add_line_mark(mark, item.origin, &item.clip)?;
                    }
                    SceneMark::Trail(mark) => {
                        self.add_trail_mark(mark, item.origin, &item.clip)?;
                    }
                    SceneMark::Area(mark) => {
                        self.add_area_mark(
                            mark,
                            item.origin,
                            &item.clip,
                            item.pattern_reference_frame.as_ref(),
                            chart_bounds,
                        )?;
                    }
                    SceneMark::Text(mark) => {
                        item_kind = "text";
                        text_mark_count += 1;
                        text_instance_count += u64::from(mark.len);
                        text_leader_count +=
                            mark.leader_iter().filter(|leader| **leader).count() as u64;
                        self.add_text_mark(mark, item.origin, &item.clip)?;
                    }
                    SceneMark::Image(mark) => {
                        item_kind = "image";
                        self.add_image_mark(mark, item.origin, &item.clip)?;
                    }

                    SceneMark::Group(_) => {}
                },
            }
            let elapsed_ms = item_start.elapsed().as_secs_f64() * 1000.0;
            match item_kind {
                "image" => image_ms += elapsed_ms,
                "text" => text_ms += elapsed_ms,
                _ => other_ms += elapsed_ms,
            }
        }
        self.set_current_zindex(0);

        tracing::debug!(
            target: "avenger_wgpu::resize",
            set_scene_ms = start.elapsed().as_secs_f64() * 1000.0,
            clear_ms = clear_elapsed.as_secs_f64() * 1000.0,
            display_list_ms = display_list_elapsed.as_secs_f64() * 1000.0,
            image_ms,
            text_ms,
            other_ms,
            text_mark_count,
            text_instance_count,
            text_leader_count,
            item_count,
            scene_width = scene_graph.width,
            scene_height = scene_graph.height,
            "wgpu.set_scene"
        );
        self.finish_scene();
        Ok(())
    }
}

fn symbol_mark_is_instanced_eligible(mark: &SceneSymbolMark, group_clip: &Clip) -> bool {
    mark.len >= 100
        && mark.gradients.is_empty()
        && is_no_fill_pattern(&mark.fill_pattern)
        && matches!(group_clip, Clip::None | Clip::Rect { .. })
}

pub(crate) fn make_wgpu_instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    })
}

pub(crate) async fn make_wgpu_adapter(
    instance: &wgpu::Instance,
    compatible_surface: Option<&Surface<'_>>,
) -> Result<Adapter, AvengerWgpuError> {
    let primary_options = RequestAdapterOptions {
        power_preference: PowerPreference::default(),
        compatible_surface,
        force_fallback_adapter: false,
    };

    match instance.request_adapter(&primary_options).await {
        Ok(adapter) => return Ok(adapter),
        Err(RequestAdapterError::NotFound { .. }) => {}
        Err(_) => return Err(AvengerWgpuError::MakeWgpuAdapterError),
    }

    let fallback_options = RequestAdapterOptions {
        power_preference: PowerPreference::LowPower,
        compatible_surface,
        force_fallback_adapter: true,
    };

    match instance.request_adapter(&fallback_options).await {
        Ok(adapter) => Ok(adapter),
        Err(_) => Err(AvengerWgpuError::MakeWgpuAdapterError),
    }
}

pub(crate) async fn request_wgpu_device(
    adapter: &Adapter,
) -> Result<(Device, Queue), AvengerWgpuError> {
    Ok(adapter
        .request_device(&DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            // WebGL doesn't support all of wgpu's features, so if
            // we're building for the web we'll have to disable some.
            required_limits: if cfg!(target_arch = "wasm32") {
                wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                wgpu::Limits::default()
            },
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: Trace::Off,
        })
        .await?)
}

pub(crate) fn create_multisampled_framebuffer(
    device: &Device,
    width: u32,
    height: u32,
    format: TextureFormat,
    sample_count: u32,
) -> TextureView {
    let multisampled_texture_extent = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let multisampled_frame_descriptor = &TextureDescriptor {
        size: multisampled_texture_extent,
        mip_level_count: 1,
        sample_count,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT,
        label: None,
        view_formats: &[],
    };

    device
        .create_texture(multisampled_frame_descriptor)
        .create_view(&TextureViewDescriptor::default())
}

pub(crate) fn get_supported_sample_count(sample_flags: TextureFormatFeatureFlags) -> u32 {
    // Get max supported sample count up to 4
    if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4) {
        4
    } else if sample_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X2) {
        2
    } else {
        1
    }
}

pub(crate) fn select_sample_count(
    sample_flags: TextureFormatFeatureFlags,
    requested: Option<u32>,
    default_sample_count: u32,
) -> u32 {
    let supported = get_supported_sample_count(sample_flags);
    let requested = requested.unwrap_or(default_sample_count).max(1);
    if requested >= 4 && supported >= 4 {
        4
    } else if requested >= 2 && supported >= 2 {
        2
    } else {
        1
    }
}

pub struct WindowCanvas<'window> {
    surface_config: SurfaceConfiguration,
    renderer: AvengerRendererCore,
    frame_overlay: Option<CanvasFrameOverlay>,

    // Order of properties determines drop order.
    // Device must be dropped after the buffers and textures associated with marks
    multisampled_framebuffer: TextureView,
    queue: Queue,
    device: Device,
    surface: Surface<'window>,
    window: Arc<Window>,
}

impl WindowCanvas<'_> {
    pub async fn new(
        window: Window,
        dimensions: CanvasDimensions,
        config: CanvasConfig,
    ) -> Result<Self, AvengerWgpuError> {
        let requested_size = dimensions.to_physical_size();
        let accepted_size = window
            .request_inner_size(Size::Physical(requested_size))
            .map(|accepted| window_accepted_or_requested_size(requested_size, accepted))
            .unwrap_or_else(|| {
                window_accepted_or_requested_size(requested_size, window.inner_size())
            });
        let dimensions = CanvasDimensions {
            size: [
                accepted_size.width as f32 / dimensions.scale,
                accepted_size.height as f32 / dimensions.scale,
            ],
            scale: dimensions.scale,
        };
        let instance = make_wgpu_instance();
        let window = Arc::new(window);
        let surface = instance.create_surface(window.clone())?;
        let adapter = make_wgpu_adapter(&instance, Some(&surface)).await?;
        let (device, queue) = request_wgpu_device(&adapter).await?;

        let surface_caps = surface.get_capabilities(&adapter);

        // Select first non-srgb texture format
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: dimensions.to_physical_width(),
            height: dimensions.to_physical_height(),
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let format_flags = adapter.get_texture_format_features(surface_format).flags;
        let sample_count = select_sample_count(format_flags, config.sample_count, 1);
        let multisampled_framebuffer = create_multisampled_framebuffer(
            &device,
            surface_config.width,
            surface_config.height,
            surface_format,
            sample_count,
        );

        // // Uncomment to capture GPU boundary
        // unsafe { device.start_graphics_debugger_capture() };

        let renderer =
            AvengerRendererCore::new(&device, dimensions, surface_format, sample_count, config);

        Ok(Self {
            surface,
            device,
            queue,
            multisampled_framebuffer,
            surface_config,
            window,
            renderer,
            frame_overlay: None,
        })
    }

    pub fn get_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.renderer.dimensions().to_physical_size()
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn set_frame_overlay(&mut self, overlay: Option<CanvasFrameOverlay>) {
        self.frame_overlay = overlay;
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.update_physical_size(new_size.width, new_size.height);
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    fn update_physical_size(&mut self, width: u32, height: u32) {
        let scale = self.renderer.dimensions().scale;
        self.renderer.set_dimensions(CanvasDimensions {
            size: [width as f32 / scale, height as f32 / scale],
            scale,
        });

        self.surface_config.width = width;
        self.surface_config.height = height;
        self.multisampled_framebuffer = create_multisampled_framebuffer(
            &self.device,
            width,
            height,
            self.surface_config.format,
            self.renderer.sample_count(),
        );
    }

    fn sync_to_acquired_surface_texture(&mut self, texture_extent: Extent3d) {
        if texture_extent.width == 0 || texture_extent.height == 0 {
            return;
        }

        if self.surface_config.width == texture_extent.width
            && self.surface_config.height == texture_extent.height
        {
            return;
        }

        self.update_physical_size(texture_extent.width, texture_extent.height);
    }

    #[allow(unused_variables)]
    pub fn input(&mut self, event: &WindowEvent) -> bool {
        false
    }

    pub fn update(&mut self) {}

    pub fn render(&mut self) -> Result<(), AvengerWgpuError> {
        let _span = tracing::debug_span!("wgpu.render").entered();
        let render_start = Instant::now();
        let output = self.surface.get_current_texture()?;
        let output_extent = output.texture.size();
        self.sync_to_acquired_surface_texture(output_extent);
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let sample_count = self.renderer.sample_count();
        let render_target_extent = if sample_count > 1 {
            Extent3d {
                width: self.surface_config.width,
                height: self.surface_config.height,
                depth_or_array_layers: 1,
            }
        } else {
            output_extent
        };

        self.renderer.commit_all_multi_renderers();
        let marks = self.renderer.marks().to_vec();

        let zindices: Vec<i32> = marks.iter().map(|m| m.zindex).collect();
        let layers = if zindices.is_empty() {
            vec![]
        } else {
            compute_zindex_layers(zindices)
        };
        let layer_count = layers.len();
        let (instanced_renderer_count, multi_renderer_count) = mark_renderer_counts(&marks);

        let command_build_start = Instant::now();
        let render_target = if sample_count > 1 {
            AvengerRenderTarget::multisampled(
                &self.multisampled_framebuffer,
                &view,
                render_target_extent,
                self.renderer.texture_format(),
                sample_count,
                WHITE_CLEAR,
            )
        } else {
            AvengerRenderTarget::swapchain(
                &view,
                render_target_extent,
                self.renderer.texture_format(),
                WHITE_CLEAR,
            )
        };
        let commands = self.renderer.build_frame_commands(
            &self.device,
            &self.queue,
            render_target,
            self.frame_overlay,
        )?;

        let command_build_elapsed = command_build_start.elapsed();
        let command_count = commands.len();
        let submit_start = Instant::now();
        self.queue.submit(commands);
        output.present();
        let submit_elapsed = submit_start.elapsed();

        tracing::debug!(
            target: "avenger_wgpu::resize",
            render_ms = render_start.elapsed().as_secs_f64() * 1000.0,
            command_build_ms = command_build_elapsed.as_secs_f64() * 1000.0,
            submit_present_ms = submit_elapsed.as_secs_f64() * 1000.0,
            command_count,
            renderer_count = marks.len(),
            instanced_renderer_count,
            multi_renderer_count,
            layer_count,
            "wgpu.render"
        );

        Ok(())
    }
}

impl Canvas for WindowCanvas<'_> {
    fn set_current_zindex(&mut self, zindex: i32) {
        self.renderer.set_current_zindex(zindex);
    }

    fn commit_multi_renderer_if_needed(&mut self, _new_zindex: i32) {
        self.renderer.commit_multi_renderer_if_needed();
    }

    fn get_current_zindex(&self) -> i32 {
        self.renderer.current_zindex()
    }

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.renderer.shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        self.renderer.text_atlas_builder_mut()
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.renderer.get_instanced_renderer(fingerprint)
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        self.renderer.add_instanced_mark_renderer(
            mark_renderer,
            fingerprint,
            x_adjustment,
            y_adjustment,
        );
    }

    fn clear_mark_renderer(&mut self) {
        self.renderer.clear_mark_renderer();
    }

    fn begin_scene(&mut self, scene: &avenger_scenegraph::scene_graph::SceneGraph) {
        self.renderer.begin_scene(scene);
    }

    fn finish_scene(&mut self) {
        self.renderer.finish_scene();
    }

    fn device(&self) -> &Device {
        &self.device
    }

    fn queue(&self) -> &Queue {
        &self.queue
    }

    fn dimensions(&self) -> CanvasDimensions {
        self.renderer.dimensions()
    }

    fn font_resolution(&self) -> &FontResolutionOptions {
        self.renderer.font_resolution()
    }

    fn text_engine(&self) -> avenger_text::TextEngine {
        self.renderer.text_engine()
    }

    fn texture_format(&self) -> TextureFormat {
        self.renderer.texture_format()
    }

    fn sample_count(&self) -> u32 {
        self.renderer.sample_count()
    }
}

// impl<'window> Drop for WindowCanvas<'window> {
//     fn drop(&mut self) {
//         unsafe { self.device.stop_graphics_debugger_capture() };
//     }
// }

pub struct PngCanvas {
    renderer: AvengerRendererCore,
    output_target: OffscreenTarget,
    readback: TextureReadback,

    // The order of properties in a struct is the order in which items are dropped.
    // wgpu seems to require that the device be dropped last, otherwise there is a resouce
    // leak.
    multisampled_framebuffer: TextureView,
    queue: Queue,
    device: Device,
}

impl PngCanvas {
    #[tracing::instrument(skip_all)]
    pub async fn new(
        dimensions: CanvasDimensions,
        config: CanvasConfig,
    ) -> Result<Self, AvengerWgpuError> {
        let instance = make_wgpu_instance();
        let adapter = make_wgpu_adapter(&instance, None).await?;
        let (device, queue) = request_wgpu_device(&adapter).await?;
        let texture_format = TextureFormat::Rgba8Unorm;
        let format_flags = adapter.get_texture_format_features(texture_format).flags;
        let sample_count = select_sample_count(
            format_flags,
            config.sample_count,
            get_supported_sample_count(format_flags),
        );
        let output_target_descriptor = OffscreenTargetDescriptor::new(dimensions, texture_format)
            .with_usage(TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT)
            .with_label("PngCanvas output texture");
        let output_target = OffscreenTarget::new(&device, &output_target_descriptor, 1);
        let readback = TextureReadback::new(&device, output_target.extent);

        let multisampled_framebuffer = create_multisampled_framebuffer(
            &device,
            output_target.extent.width,
            output_target.extent.height,
            texture_format,
            sample_count,
        );

        let renderer =
            AvengerRendererCore::new(&device, dimensions, texture_format, sample_count, config);

        Ok(Self {
            device,
            queue,
            multisampled_framebuffer,
            renderer,
            output_target,
            readback,
        })
    }

    #[tracing::instrument(skip_all)]
    pub async fn render(&mut self) -> Result<image::RgbaImage, AvengerWgpuError> {
        let render_start = Instant::now();
        self.renderer.commit_all_multi_renderers();
        let sample_count = self.renderer.sample_count();
        let dimensions = self.renderer.dimensions();
        let marks = self.renderer.marks().to_vec();

        let zindices: Vec<i32> = marks.iter().map(|m| m.zindex).collect();
        let layers = if zindices.is_empty() {
            vec![]
        } else {
            compute_zindex_layers(zindices)
        };
        let layer_count = layers.len();
        let (instanced_renderer_count, multi_renderer_count) = mark_renderer_counts(&marks);

        let render_target_extent = self.output_target.extent;
        let render_target = if sample_count > 1 {
            AvengerRenderTarget::multisampled(
                &self.multisampled_framebuffer,
                &self.output_target.view,
                render_target_extent,
                self.renderer.texture_format(),
                sample_count,
                WHITE_CLEAR,
            )
        } else {
            self.output_target.render_target(WHITE_CLEAR)
        };

        let command_build_start = Instant::now();
        let commands =
            self.renderer
                .build_frame_commands(&self.device, &self.queue, render_target, None)?;
        let command_build_elapsed = command_build_start.elapsed();
        let command_count = commands.len();

        let submit_start = Instant::now();
        self.queue.submit(commands);
        let submit_elapsed = submit_start.elapsed();

        let extract_start = Instant::now();
        let mut extract_encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Extract Texture Encoder"),
            });
        debug_assert_eq!(self.readback.texture_extent(), self.output_target.extent);
        self.readback
            .encode_copy_from_texture(&mut extract_encoder, &self.output_target.texture);
        self.queue.submit(Some(extract_encoder.finish()));
        let extract_elapsed = extract_start.elapsed();

        let map_read_start = Instant::now();
        let img = self
            .readback
            .read_rgba8(
                &self.device,
                dimensions.to_physical_width(),
                dimensions.to_physical_height(),
            )
            .await?;
        let map_read_elapsed = map_read_start.elapsed();

        tracing::debug!(
            target: "avenger_wgpu::resize",
            render_ms = render_start.elapsed().as_secs_f64() * 1000.0,
            command_build_ms = command_build_elapsed.as_secs_f64() * 1000.0,
            submit_ms = submit_elapsed.as_secs_f64() * 1000.0,
            extract_ms = extract_elapsed.as_secs_f64() * 1000.0,
            map_read_ms = map_read_elapsed.as_secs_f64() * 1000.0,
            physical_width = dimensions.to_physical_width(),
            physical_height = dimensions.to_physical_height(),
            command_count,
            renderer_count = marks.len(),
            instanced_renderer_count,
            multi_renderer_count,
            layer_count,
            "png.render"
        );
        Ok(img)
    }
}

impl Canvas for PngCanvas {
    fn set_current_zindex(&mut self, zindex: i32) {
        self.renderer.set_current_zindex(zindex);
    }

    fn commit_multi_renderer_if_needed(&mut self, _new_zindex: i32) {
        self.renderer.commit_multi_renderer_if_needed();
    }

    fn get_current_zindex(&self) -> i32 {
        self.renderer.current_zindex()
    }

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.renderer.shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        self.renderer.text_atlas_builder_mut()
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.renderer.get_instanced_renderer(fingerprint)
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        self.renderer.add_instanced_mark_renderer(
            mark_renderer,
            fingerprint,
            x_adjustment,
            y_adjustment,
        );
    }

    fn clear_mark_renderer(&mut self) {
        self.renderer.clear_mark_renderer();
    }

    fn begin_scene(&mut self, scene: &avenger_scenegraph::scene_graph::SceneGraph) {
        self.renderer.begin_scene(scene);
    }

    fn finish_scene(&mut self) {
        self.renderer.finish_scene();
    }

    fn device(&self) -> &Device {
        &self.device
    }

    fn queue(&self) -> &Queue {
        &self.queue
    }

    fn dimensions(&self) -> CanvasDimensions {
        self.renderer.dimensions()
    }

    fn font_resolution(&self) -> &FontResolutionOptions {
        self.renderer.font_resolution()
    }

    fn text_engine(&self) -> avenger_text::TextEngine {
        self.renderer.text_engine()
    }

    fn texture_format(&self) -> TextureFormat {
        self.renderer.texture_format()
    }

    fn sample_count(&self) -> u32 {
        self.renderer.sample_count()
    }
}

#[cfg(test)]
mod tests {
    use avenger_color::{Gradient, GradientStop, LinearGradient};
    use avenger_common::value::ScalarOrArray;
    use avenger_scenegraph::marks::{
        group::Clip,
        pattern::{PatternAnchor, PatternFill, PatternLayer, StripePatternLayer},
        symbol::SceneSymbolMark,
    };

    use super::symbol_mark_is_instanced_eligible;

    fn large_symbol_mark() -> SceneSymbolMark {
        SceneSymbolMark {
            len: 100,
            ..Default::default()
        }
    }

    #[test]
    fn large_symbol_mark_with_no_clip_is_instanced_eligible() {
        assert!(symbol_mark_is_instanced_eligible(
            &large_symbol_mark(),
            &Clip::None
        ));
    }

    #[test]
    fn stroked_symbol_mark_is_instanced_eligible() {
        let mark = SceneSymbolMark {
            stroke_width: Some(2.0),
            ..large_symbol_mark()
        };

        assert!(symbol_mark_is_instanced_eligible(&mark, &Clip::None));
    }

    #[test]
    fn gradient_symbol_mark_is_not_instanced_eligible() {
        let mark = SceneSymbolMark {
            gradients: vec![Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: [0.0, 0.0, 0.0, 1.0],
                    },
                    GradientStop {
                        offset: 1.0,
                        color: [1.0, 1.0, 1.0, 1.0],
                    },
                ],
            })],
            ..large_symbol_mark()
        };

        assert!(!symbol_mark_is_instanced_eligible(&mark, &Clip::None));
    }

    #[test]
    fn path_clipped_symbol_mark_is_not_instanced_eligible() {
        assert!(!symbol_mark_is_instanced_eligible(
            &large_symbol_mark(),
            &Clip::Path(lyon::path::Path::default()),
        ));
    }

    #[test]
    fn patterned_symbol_mark_is_not_instanced_eligible() {
        let mark = SceneSymbolMark {
            fill_pattern: ScalarOrArray::new_scalar(Some(PatternFill {
                anchor: PatternAnchor::Mark,
                layers: vec![PatternLayer::Stripe(StripePatternLayer::new(0.0, 8.0, 2.0))],
                ..Default::default()
            })),
            ..large_symbol_mark()
        };

        assert!(!symbol_mark_is_instanced_eligible(&mark, &Clip::None));
    }
}
