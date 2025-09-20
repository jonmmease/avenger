//! Main plot renderer orchestration
//!
//! This module handles the high-level rendering workflow:
//! - Building initial scales with estimated dimensions
//! - Computing layout
//! - Rebuilding scales with final dimensions
//! - Rendering all components
//! - Composing the final scene graph

use super::PlotRenderer;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::plot::Plot;
use crate::render::types::INITIAL_PLOT_AREA_RATIO;
use crate::render::{LayoutSolution, RenderContext, RenderResult};
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use std::any::Any;
use std::collections::HashMap;

impl<'a, C: CoordinateSystem + Any> PlotRenderer<'a, C> {
    pub fn new(plot: &'a Plot<C>) -> Self {
        Self { plot }
    }

    /// Render the plot to a scene graph
    pub async fn render(&self) -> Result<RenderResult, AvengerChartError> {
        // Get plot dimensions from preferred size or default
        let (width, height) = self.plot.get_preferred_size().unwrap_or((400.0, 300.0));

        // STAGE 1: BUILD INITIAL SCALES WITH ESTIMATED DIMENSIONS
        // Use estimated dimensions for initial scale construction
        let estimated_plot_width = width * INITIAL_PLOT_AREA_RATIO;
        let estimated_plot_height = height * INITIAL_PLOT_AREA_RATIO;

        // Create initial RenderContext with estimated dimensions
        let theme = self.plot.get_theme();
        let initial_context =
            RenderContext::new(theme.clone(), estimated_plot_width, estimated_plot_height);

        let (initial_scales, configured_non_positional, configured_positional) =
            self.build_initial_scales(&initial_context).await?;

        // Merge configured scales for layout computation
        let mut initial_configured_scales = configured_non_positional.clone();
        initial_configured_scales.extend(configured_positional.clone());

        // STAGE 2: COMPUTE LAYOUT USING INITIAL SCALES
        let layout = self
            .compute_layout(width, height, &initial_configured_scales)
            .await?;
        let plot_bounds = layout.plot_area_bounds();
        let plot_area_x = plot_bounds.x;
        let plot_area_y = plot_bounds.y;
        let plot_area_width = plot_bounds.width;
        let plot_area_height = plot_bounds.height;

        // STAGE 3: REBUILD POSITIONAL SCALES WITH FINAL DIMENSIONS
        // Create final RenderContext with actual plot dimensions
        let final_context = RenderContext::new(theme.clone(), plot_area_width, plot_area_height);

        let final_configured_scales = self
            .rebuild_scales_with_final_dimensions(
                &initial_scales,
                &configured_non_positional,
                &final_context,
            )
            .await?;

        // STAGE 4: RENDER ALL COMPONENTS WITH FINAL SCALES
        let all_component_marks = self
            .render_all_components(&final_configured_scales, &layout, width, height)
            .await?;

        let (mark_groups, guide_marks, legend_marks, title_marks, subtitle_marks) =
            all_component_marks;

        // Compose all elements into a scene graph
        // A single Plot should produce a single top-level group
        let mut all_marks = Vec::new();

        // Get the appropriate clipping region from the coordinate system
        let clip = self.plot.coord_system().get_clip(
            plot_area_width,
            plot_area_height,
            &final_configured_scales,
        );

        let data_marks_group = SceneGroup {
            origin: [plot_area_x, plot_area_y],
            marks: mark_groups,
            clip,
            zindex: Some(0), // Data marks have lowest z-index
            ..Default::default()
        };

        // Add background rect if theme specifies one
        let theme = self.plot.get_theme();
        if let Some(bg_color) = theme.canvas_background() {
            use avenger_common::types::ColorOrGradient;
            use avenger_scenegraph::marks::rect::SceneRectMark;

            // Parse the color string to RGBA - fail if color is invalid
            let color = crate::utils::parse_color_to_array_strict(&bg_color)?;

            let background_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(width.into()),
                height: Some(height.into()),
                fill: ColorOrGradient::Color(color).into(),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(), // No stroke
                stroke_width: 0.0.into(),
                zindex: Some(-100), // Ensure it's behind everything
                ..Default::default()
            };
            all_marks.push(SceneMark::Rect(background_rect));
        }

        // Add marks in proper z-order:
        // 1. Clipped data marks (background)
        all_marks.push(SceneMark::Group(data_marks_group));

        // 2. Guide marks (axes, grids, backgrounds - can overflow the plot area)
        all_marks.extend(guide_marks);

        // 3. Legends (positioned outside plot area)
        all_marks.extend(legend_marks);

        // 4. Title (can overflow, rendered on top)
        all_marks.extend(title_marks);

        // 5. Subtitle (can overflow, rendered on top)
        all_marks.extend(subtitle_marks);

        // 6. Debug: Add Taffy layout bounds visualization if AVENGER_CHART_DEBUG_LAYOUT is set
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            all_marks.extend(super::debug::create_debug_layout_rects(
                &layout.taffy_layout,
            ));
        }

        // Wrap everything in a single root group
        let root_group = SceneGroup {
            marks: all_marks,
            ..Default::default()
        };

        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(root_group)],
            width,
            height,
            origin: [0.0, 0.0],
        };

        // Build spatial index for hit testing
        let rtree = avenger_geometry::rtree::SceneGraphRTree::from_scene_graph(&scene_graph);

        Ok(RenderResult {
            scene_graph,
            rtree: Some(rtree),
        })
    }

    /// Render all components (marks, axes, legends, titles)
    async fn render_all_components(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        layout: &LayoutSolution,
        width: f32,
        _height: f32,
    ) -> Result<
        (
            Vec<SceneMark>, // mark_groups
            Vec<SceneMark>, // axis_marks
            Vec<SceneMark>, // legend_marks
            Vec<SceneMark>, // title_marks
            Vec<SceneMark>, // subtitle_marks
        ),
        AvengerChartError,
    > {
        let plot_bounds = layout.plot_area_bounds();
        let plot_area_width = plot_bounds.width;
        let plot_area_height = plot_bounds.height;

        // Render marks
        let mut mark_groups = Vec::new();
        for mark in &self.plot.marks {
            let scene_marks = self
                .render_mark(mark.as_ref(), scales, plot_area_width, plot_area_height)
                .await?;
            mark_groups.extend(scene_marks);
        }

        // Create guide marks (axes, grids, backgrounds)
        let guide_marks = self
            .create_guide_marks(scales, plot_area_width, plot_area_height, plot_bounds)
            .await?;

        // Create legends
        let legend_marks = self
            .create_legends_with_layout(
                scales,
                &layout.taffy_layout,
                plot_area_width,
                plot_area_height,
            )
            .await?;

        // Create title
        let title_marks = if let Some(title_bounds) = &layout.taffy_layout.title {
            self.create_title(
                width,
                plot_bounds,
                Some(*title_bounds),
                Some(layout.taffy_layout.plot_area),
            )?
        } else {
            Vec::new()
        };

        // Create subtitle
        let subtitle_marks = if let Some(subtitle_bounds) = &layout.taffy_layout.subtitle {
            self.create_subtitle(
                width,
                plot_bounds,
                Some(*subtitle_bounds),
                Some(layout.taffy_layout.plot_area),
            )?
        } else {
            Vec::new()
        };

        Ok((
            mark_groups,
            guide_marks,
            legend_marks,
            title_marks,
            subtitle_marks,
        ))
    }
}
