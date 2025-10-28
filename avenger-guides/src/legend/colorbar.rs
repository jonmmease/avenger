use avenger_common::types::{ColorOrGradient, Gradient, LinearGradient};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, rect::SceneRectMark};
use avenger_text::measurement::{default_text_measurer, TextMeasurementConfig, TextMeasurer};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec};

use crate::{
    axis::{
        numeric::make_numeric_axis_marks,
        opts::{AxisConfig, AxisOrientation},
    },
    error::AvengerGuidesError,
};

pub fn make_colorbar_marks(
    scale: &ConfiguredScale,
    title: &str,
    _origin: [f32; 2], // Unused - we always start at (0, 0) now
    config: &ColorbarConfig,
) -> Result<SceneGroup, AvengerGuidesError> {
    match config.orientation {
        ColorbarOrientation::Top => {
            // Horizontal gradient going left-to-right with axis above
            let available_width = config.dimensions[0];
            let _available_height = config.dimensions[1];

            let total_width = config.colorbar_height.unwrap_or(available_width.min(200.0));
            let bg_padding = config.background_padding.unwrap_or(4.0);

            // For horizontal colorbars (Top/Bottom), measure tick label widths to reserve horizontal space
            // This ensures tick labels at left/right don't run into the background edge
            let measurer = default_text_measurer();
            let label_font_size = config.label_font_size.unwrap_or(10.0);
            let label_font_weight = config
                .label_font_weight
                .as_ref()
                .unwrap_or(&FontWeight::Number(400.0));
            let label_font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");

            // Get the domain min and max values to measure their formatted width
            let (domain_min, domain_max) = scale.config.numeric_interval_domain()?;

            // Format the min and max values using the format spec if provided
            let formatter = avenger_scales::format_num::NumberFormat::new();
            let min_label = if let Some(format_spec) = &config.format_number {
                formatter.format(format_spec, domain_min as f64)
            } else {
                domain_min.to_string()
            };
            let max_label = if let Some(format_spec) = &config.format_number {
                formatter.format(format_spec, domain_max as f64)
            } else {
                domain_max.to_string()
            };

            // Measure both labels and take the maximum width
            let min_bounds = measurer.measure_text_bounds(&TextMeasurementConfig {
                text: &min_label,
                font: label_font_family,
                font_size: label_font_size,
                font_weight: label_font_weight,
                font_style: &FontStyle::Normal,
            });
            let max_bounds = measurer.measure_text_bounds(&TextMeasurementConfig {
                text: &max_label,
                font: label_font_family,
                font_size: label_font_size,
                font_weight: label_font_weight,
                font_style: &FontStyle::Normal,
            });

            // Calculate how much the labels might overflow beyond gradient edges
            let text_overflow = (min_bounds.width.max(max_bounds.width) / 2.0).round();

            // Only reduce gradient if text overflow exceeds available padding
            // Keep a minimum margin between text and background edge
            let min_margin = 2.0;
            let horizontal_text_padding =
                ((text_overflow - (bg_padding - min_margin)).max(0.0)).round();

            // Calculate the actual gradient width by subtracting padding and text padding
            let gradient_width = ((total_width - 2.0 * bg_padding - 2.0 * horizontal_text_padding)
                .max(10.0))
            .round();
            let colorbar_height = config.colorbar_width.unwrap_or(15.0).round();
            let colorbar_margin = config.colorbar_margin.unwrap_or(5.0).round();

            // Create horizontal gradient (left to right)
            let gradient = Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: 0.0,
                x1: gradient_width,
                y1: 0.0,
                stops: scale.color_range_as_gradient_stops(10)?,
            });

            // Create axis configuration for top orientation
            let axis_config = AxisConfig {
                orientation: AxisOrientation::Top,
                dimensions: [gradient_width, 0.0],
                grid: false,
                format_number: config.format_number.clone(),
                title_font_size: config.title_font_size,
                title_font_weight: config.title_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                title_font_family: config.title_font_family.clone(),
                title_color: config.title_color,
                label_font_size: config.label_font_size,
                label_font_weight: config.label_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                label_font_family: config.label_font_family.clone(),
                label_color: config.label_color,
                domain_color: config.domain_color,
                tick_color: config.tick_color,
                grid_color: None,
                grid_width: None,
                tick_length: None,
                title_visible: Some(true),
                labels_visible: None,
            };

            let numeric_scale = scale.clone().with_range_interval((0.0, gradient_width));

            // Position colorbar rect at y=0
            let rect = SceneRectMark {
                len: 1,
                gradients: vec![gradient],
                x: 0.0.into(),
                x2: Some(gradient_width.into()),
                y: 0.0.into(),
                y2: Some(colorbar_height.into()),
                fill: ColorOrGradient::GradientIndex(0).into(),
                clip: false,
                ..Default::default()
            };

            // Position axis above the colorbar
            // For Top orientation, the axis renders upward from its origin
            // Place the axis origin at the top of the rect, minus margin
            let axis_origin = [0.0, -colorbar_margin];
            let axis = make_numeric_axis_marks(&numeric_scale, title, axis_origin, &axis_config)?;

            // Content marks
            let content_marks = vec![axis.clone().into(), rect.clone().into()];
            let content_group = SceneGroup {
                marks: content_marks.clone(),
                ..Default::default()
            };
            let content_bbox = content_group.bounding_box();

            // For Top axis, the content may extend into negative y if axis is above
            // Use the actual bounding box to determine height, but shift everything if needed
            let min_y = content_bbox.lower()[1];
            let max_y = content_bbox.upper()[1];
            let content_height = max_y - min_y;

            // If min_y is negative, we need to shift all content down by that amount
            let y_shift = if min_y < 0.0 { -min_y } else { 0.0 };

            // Background dimensions (horizontal_text_padding already accounted for in gradient_width)
            let bg_width = total_width.round();
            let bg_height = (content_height + bg_padding * 2.0).round();

            let bg_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(bg_width.into()),
                height: Some(bg_height.into()),
                fill: config
                    .background_fill
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]))
                    .into(),
                stroke: config
                    .background_stroke
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]))
                    .into(),
                stroke_width: if config.background_stroke.is_some() {
                    1.0.into()
                } else {
                    0.0.into()
                },
                corner_radius: config.background_corner_radius.unwrap_or(0.0).into(),
                zindex: Some(0),
                ..Default::default()
            };

            // Shift content to account for any negative y overhang, then add bg_padding
            // Add horizontal_text_padding to ensure equal spacing for tick labels on left/right
            let colorbar_axis_group = SceneGroup {
                origin: [horizontal_text_padding + bg_padding, y_shift + bg_padding],
                marks: vec![axis.into(), rect.into()],
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            };

            let marks = vec![bg_rect.into(), colorbar_axis_group.into()];

            Ok(SceneGroup {
                origin: [0.0, 0.0],
                marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            })
        }
        ColorbarOrientation::Bottom => {
            // Horizontal gradient going left-to-right with axis below
            let available_width = config.dimensions[0];
            let _available_height = config.dimensions[1];

            // For horizontal colorbars, width is the length and colorbar_width becomes the thickness
            let total_width = config
                .colorbar_height // Using colorbar_height for the length dimension
                .unwrap_or(available_width.min(200.0));
            let bg_padding = config.background_padding.unwrap_or(4.0);

            // For horizontal colorbars (Top/Bottom), measure tick label widths to reserve horizontal space
            // This ensures tick labels at left/right don't run into the background edge
            let measurer = default_text_measurer();
            let label_font_size = config.label_font_size.unwrap_or(10.0);
            let label_font_weight = config
                .label_font_weight
                .as_ref()
                .unwrap_or(&FontWeight::Number(400.0));
            let label_font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");

            // Get the domain min and max values to measure their formatted width
            let (domain_min, domain_max) = scale.config.numeric_interval_domain()?;

            // Format the min and max values using the format spec if provided
            let formatter = avenger_scales::format_num::NumberFormat::new();
            let min_label = if let Some(format_spec) = &config.format_number {
                formatter.format(format_spec, domain_min as f64)
            } else {
                domain_min.to_string()
            };
            let max_label = if let Some(format_spec) = &config.format_number {
                formatter.format(format_spec, domain_max as f64)
            } else {
                domain_max.to_string()
            };

            // Measure both labels and take the maximum width
            let min_bounds = measurer.measure_text_bounds(&TextMeasurementConfig {
                text: &min_label,
                font: label_font_family,
                font_size: label_font_size,
                font_weight: label_font_weight,
                font_style: &FontStyle::Normal,
            });
            let max_bounds = measurer.measure_text_bounds(&TextMeasurementConfig {
                text: &max_label,
                font: label_font_family,
                font_size: label_font_size,
                font_weight: label_font_weight,
                font_style: &FontStyle::Normal,
            });

            // Calculate how much the labels might overflow beyond gradient edges
            let text_overflow = (min_bounds.width.max(max_bounds.width) / 2.0).round();

            // Only reduce gradient if text overflow exceeds available padding
            // Keep a minimum margin between text and background edge
            let min_margin = 2.0;
            let horizontal_text_padding =
                ((text_overflow - (bg_padding - min_margin)).max(0.0)).round();

            // Calculate the actual gradient width by subtracting padding and text padding
            let gradient_width = ((total_width - 2.0 * bg_padding - 2.0 * horizontal_text_padding)
                .max(10.0))
            .round();
            let colorbar_height = config.colorbar_width.unwrap_or(15.0).round(); // thickness
            let colorbar_margin = config.colorbar_margin.unwrap_or(5.0).round();

            // Create horizontal gradient (left to right)
            let gradient = Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: 0.0,
                x1: gradient_width,
                y1: 0.0,
                stops: scale.color_range_as_gradient_stops(10)?,
            });

            // Make colorbar rect
            let rect = SceneRectMark {
                len: 1,
                gradients: vec![gradient],
                x: 0.0.into(),
                x2: Some(gradient_width.into()),
                y: 0.0.into(),
                y2: Some(colorbar_height.into()),
                fill: ColorOrGradient::GradientIndex(0).into(),
                clip: false,
                ..Default::default()
            };

            // Create axis configuration for bottom orientation
            let axis_config = AxisConfig {
                orientation: AxisOrientation::Bottom,
                dimensions: [gradient_width, 0.0],
                grid: false,
                format_number: config.format_number.clone(),
                title_font_size: config.title_font_size,
                title_font_weight: config.title_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                title_font_family: config.title_font_family.clone(),
                title_color: config.title_color,
                label_font_size: config.label_font_size,
                label_font_weight: config.label_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                label_font_family: config.label_font_family.clone(),
                label_color: config.label_color,
                domain_color: config.domain_color,
                tick_color: config.tick_color,
                grid_color: None,
                grid_width: None,
                tick_length: None,
                title_visible: Some(true),
                labels_visible: None,
            };

            // Create scale with horizontal range
            let numeric_scale = scale.clone().with_range_interval((0.0, gradient_width));

            // Position axis below the colorbar
            let axis_origin = [0.0, colorbar_height + colorbar_margin];
            let axis = make_numeric_axis_marks(&numeric_scale, title, axis_origin, &axis_config)?;

            // Content marks
            let content_marks = vec![rect.clone().into(), axis.clone().into()];
            let content_group = SceneGroup {
                marks: content_marks.clone(),
                ..Default::default()
            };
            let content_bbox = content_group.bounding_box();

            // Calculate background dimensions to ensure equal padding on all sides
            let min_y = content_bbox.lower()[1];
            let max_y = content_bbox.upper()[1];
            let content_height = max_y - min_y;

            // If content extends into negative y, shift everything down
            let y_shift = if min_y < 0.0 { -min_y } else { 0.0 };

            let bg_width = total_width.round();
            let bg_height = (content_height + bg_padding * 2.0).round();

            let bg_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(bg_width.into()),
                height: Some(bg_height.into()),
                fill: config
                    .background_fill
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]))
                    .into(),
                stroke: config
                    .background_stroke
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]))
                    .into(),
                stroke_width: if config.background_stroke.is_some() {
                    1.0.into()
                } else {
                    0.0.into()
                },
                corner_radius: config.background_corner_radius.unwrap_or(0.0).into(),
                zindex: Some(0),
                ..Default::default()
            };

            // Shift content down if there's negative y overhang, then add bg_padding
            // Add horizontal_text_padding to ensure equal spacing for tick labels on left/right
            let colorbar_axis_group = SceneGroup {
                origin: [horizontal_text_padding + bg_padding, y_shift + bg_padding],
                marks: vec![rect.into(), axis.into()],
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            };

            let marks = vec![bg_rect.into(), colorbar_axis_group.into()];

            Ok(SceneGroup {
                origin: [0.0, 0.0],
                marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            })
        }
        ColorbarOrientation::Left => {
            // Similar to Right but axis is positioned to the left of the gradient
            let _available_width = config.dimensions[0];
            let available_height = config.dimensions[1];

            let total_height = config
                .colorbar_height
                .unwrap_or(available_height.min(200.0));
            let bg_padding = config.background_padding.unwrap_or(4.0);

            // For vertical colorbars (Left/Right), measure tick label line height to reserve vertical space
            // This ensures tick labels at top/bottom don't run into the background edge
            let measurer = default_text_measurer();
            let label_font_size = config.label_font_size.unwrap_or(10.0);
            let label_font_weight = config
                .label_font_weight
                .as_ref()
                .unwrap_or(&FontWeight::Number(400.0));
            let label_font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");

            let text_bounds = measurer.measure_text_bounds(&TextMeasurementConfig {
                text: "0",
                font: label_font_family,
                font_size: label_font_size,
                font_weight: label_font_weight,
                font_style: &FontStyle::Normal,
            });

            // Calculate how much the labels might overflow beyond gradient edges
            let text_overflow = (text_bounds.line_height / 2.0).round();

            // Only reduce gradient if text overflow exceeds available padding
            // Keep a minimum margin between text and background edge
            let min_margin = 2.0;
            let vertical_text_padding =
                ((text_overflow - (bg_padding - min_margin)).max(0.0)).round();

            // Calculate the actual gradient height by subtracting padding and text padding
            let gradient_height =
                ((total_height - 2.0 * bg_padding - 2.0 * vertical_text_padding).max(10.0)).round();
            let colorbar_width = config.colorbar_width.unwrap_or(15.0).round();
            let colorbar_margin = config.colorbar_margin.unwrap_or(5.0).round();

            // Create a gradient for the colorbar rect (same as Right - vertical bottom-to-top)
            let gradient = Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: gradient_height,
                x1: 0.0,
                y1: 0.0,
                stops: scale.color_range_as_gradient_stops(10)?,
            });

            // First, create the axis with Left orientation to measure its width
            let axis_config = AxisConfig {
                orientation: AxisOrientation::Left,
                dimensions: [0.0, gradient_height],
                grid: false,
                format_number: config.format_number.clone(),
                title_font_size: config.title_font_size,
                title_font_weight: config.title_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                title_font_family: config.title_font_family.clone(),
                title_color: config.title_color,
                label_font_size: config.label_font_size,
                label_font_weight: config.label_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                label_font_family: config.label_font_family.clone(),
                label_color: config.label_color,
                domain_color: config.domain_color,
                tick_color: config.tick_color,
                grid_color: None,
                grid_width: None,
                tick_length: None,
                title_visible: Some(true),
                labels_visible: None,
            };

            let numeric_scale = scale.clone().with_range_interval((gradient_height, 0.0));

            // For Left axis: The axis renders to the LEFT of its origin point
            // We want: [axis content] [margin] [rect at 0]
            // So we need to find where to place the axis origin

            // First measure the axis to see its extent
            let _axis_temp =
                make_numeric_axis_marks(&numeric_scale, title, [0.0, 0.0], &axis_config)?;

            // The axis origin should be positioned so the axis ends at -colorbar_margin
            // (i.e., margin distance to the left of the rect at x=0)
            // Since the axis extends leftward from its origin, if we want it to end at x = -margin,
            // and it has width W, we need to position origin at: -margin + 0 = -margin
            // But actually, the axis renders from its origin to the left, so:
            // Origin at x = axis_width means the axis spans from 0 to axis_width (to the left)
            // We want the rect at 0, so axis origin should be at -colorbar_margin
            // But that would make the axis go from -colorbar_margin leftward...

            // Position rect at x=0 (it will be shifted by bg_padding + x_shift later for proper spacing)
            let rect = SceneRectMark {
                len: 1,
                gradients: vec![gradient],
                x: 0.0.into(),
                x2: Some(colorbar_width.into()),
                y: 0.0.into(),
                y2: Some(gradient_height.into()),
                fill: ColorOrGradient::GradientIndex(0).into(),
                clip: false,
                ..Default::default()
            };

            // Position axis to the left of rect with margin
            // Axis renders leftward from origin, so place origin at -margin (to the left of rect at x=0)
            let axis_origin = [-(colorbar_margin), 0.0];
            let axis = make_numeric_axis_marks(&numeric_scale, title, axis_origin, &axis_config)?;

            // Content marks
            let content_marks = vec![axis.clone().into(), rect.clone().into()];
            let content_group = SceneGroup {
                marks: content_marks.clone(),
                ..Default::default()
            };
            let content_bbox = content_group.bounding_box();

            // For Left axis, the content may extend into negative x if tick marks overhang
            // Use the actual bounding box to determine width, but shift everything if needed
            let min_x = content_bbox.lower()[0];
            let max_x = content_bbox.upper()[0];
            let content_width = max_x - min_x;

            // If min_x is negative, we need to shift all content to the right by that amount
            let x_shift = if min_x < 0.0 { -min_x } else { 0.0 };

            // Background dimensions (vertical_text_padding already accounted for in gradient_height)
            let bg_width = (content_width + bg_padding * 2.0).round();
            let bg_height = total_height.round();

            let bg_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(bg_width.into()),
                height: Some(bg_height.into()),
                fill: config
                    .background_fill
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]))
                    .into(),
                stroke: config
                    .background_stroke
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]))
                    .into(),
                stroke_width: if config.background_stroke.is_some() {
                    1.0.into()
                } else {
                    0.0.into()
                },
                corner_radius: config.background_corner_radius.unwrap_or(0.0).into(),
                zindex: Some(0),
                ..Default::default()
            };

            // Shift content to account for any negative x overhang, then add bg_padding
            // Add vertical_text_padding to ensure equal spacing for tick labels
            let colorbar_axis_group = SceneGroup {
                origin: [x_shift + bg_padding, vertical_text_padding + bg_padding],
                marks: vec![axis.into(), rect.into()],
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            };

            let marks = vec![bg_rect.into(), colorbar_axis_group.into()];

            Ok(SceneGroup {
                origin: [0.0, 0.0],
                marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            })
        }
        ColorbarOrientation::Right => {
            // config.dimensions represents available space for the colorbar
            let _available_width = config.dimensions[0];
            let available_height = config.dimensions[1];

            // Colorbar properties
            // colorbar_height now refers to the total height including background padding
            let total_height = config
                .colorbar_height
                .unwrap_or(available_height.min(200.0));
            let bg_padding = config.background_padding.unwrap_or(4.0);

            // For vertical colorbars (Left/Right), measure tick label line height to reserve vertical space
            // This ensures tick labels at top/bottom don't run into the background edge
            let measurer = default_text_measurer();
            let label_font_size = config.label_font_size.unwrap_or(10.0);
            let label_font_weight = config
                .label_font_weight
                .as_ref()
                .unwrap_or(&FontWeight::Number(400.0));
            let label_font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");

            let text_bounds = measurer.measure_text_bounds(&TextMeasurementConfig {
                text: "0",
                font: label_font_family,
                font_size: label_font_size,
                font_weight: label_font_weight,
                font_style: &FontStyle::Normal,
            });

            // Calculate how much the labels might overflow beyond gradient edges
            let text_overflow = (text_bounds.line_height / 2.0).round();

            // Only reduce gradient if text overflow exceeds available padding
            // Keep a minimum margin between text and background edge
            let min_margin = 2.0;
            let vertical_text_padding =
                ((text_overflow - (bg_padding - min_margin)).max(0.0)).round();

            // Calculate the actual gradient height by subtracting padding and text padding
            // Round dimensions to pixel boundaries
            let gradient_height =
                ((total_height - 2.0 * bg_padding - 2.0 * vertical_text_padding).max(10.0)).round();

            let colorbar_width = config.colorbar_width.unwrap_or(15.0).round();
            let colorbar_margin = config.colorbar_margin.unwrap_or(5.0).round();

            // Create a gradient for the colorbar rect
            let gradient = Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: gradient_height,
                x1: 0.0,
                y1: 0.0,
                stops: scale.color_range_as_gradient_stops(10)?,
            });

            // Make colorbar rect
            let rect = SceneRectMark {
                len: 1,
                gradients: vec![gradient],
                x: 0.0.into(),
                x2: Some((colorbar_width).into()),
                y: 0.0.into(),
                y2: Some((gradient_height).into()),
                fill: ColorOrGradient::GradientIndex(0).into(),
                clip: false,
                ..Default::default()
            };

            // Make axis positioned to the right of the colorbar with small margin
            let axis_origin = [colorbar_width + colorbar_margin, 0.0];
            let axis_config = AxisConfig {
                orientation: AxisOrientation::Right,
                dimensions: [0.0, gradient_height],
                grid: false,
                format_number: config.format_number.clone(),
                title_font_size: config.title_font_size,
                title_font_weight: config.title_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                title_font_family: config.title_font_family.clone(),
                title_color: config.title_color,
                label_font_size: config.label_font_size,
                label_font_weight: config.label_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                label_font_family: config.label_font_family.clone(),
                label_color: config.label_color,
                domain_color: config.domain_color,
                tick_color: config.tick_color,
                grid_color: None,
                grid_width: None,
                tick_length: None,
                title_visible: Some(true),
                labels_visible: None,
            };

            // Create a new scale with desired range for the axis
            let numeric_scale = scale.clone().with_range_interval((gradient_height, 0.0));
            let axis = make_numeric_axis_marks(&numeric_scale, title, axis_origin, &axis_config)?;

            // Content marks
            let content_marks = vec![rect.clone().into(), axis.clone().into()];
            let content_group = SceneGroup {
                marks: content_marks.clone(),
                ..Default::default()
            };
            let content_bbox = content_group.bounding_box();

            // Calculate background dimensions to ensure equal padding on all sides
            // Content will be shifted by bg_padding, so we need to account for this
            let min_x = content_bbox.lower()[0];
            let max_x = content_bbox.upper()[0];
            let content_width = max_x - min_x;

            // If content extends into negative x, shift everything right
            let x_shift = if min_x < 0.0 { -min_x } else { 0.0 };

            // Background width includes the content width plus padding on both sides
            // Background height is total_height (vertical_text_padding already accounted for in gradient_height)
            let bg_width = (content_width + bg_padding * 2.0).round();
            let bg_height = total_height.round();

            // Always create a background rect at origin (0, 0)
            // This provides consistent layout whether visible or not
            let bg_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(bg_width.into()),
                height: Some(bg_height.into()),
                fill: config
                    .background_fill
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])) // Transparent by default
                    .into(),
                stroke: config
                    .background_stroke
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])) // No stroke by default
                    .into(),
                stroke_width: if config.background_stroke.is_some() {
                    1.0.into()
                } else {
                    0.0.into()
                },
                corner_radius: config.background_corner_radius.unwrap_or(0.0).into(),
                zindex: Some(0),
                ..Default::default()
            };

            // Create a group for colorbar and axis with proper padding
            // Shift content right if there's negative x overhang, then add bg_padding
            // Add vertical_text_padding to ensure equal spacing for tick labels
            let colorbar_axis_group = SceneGroup {
                origin: [x_shift + bg_padding, vertical_text_padding + bg_padding],
                marks: vec![rect.into(), axis.into()],
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            };

            // Insert background rect first, then legend content
            let marks = vec![bg_rect.into(), colorbar_axis_group.into()];

            Ok(SceneGroup {
                origin: [0.0, 0.0],
                marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            })
        }
    }
}

#[derive(Debug, Clone)]
pub enum ColorbarOrientation {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct ColorbarConfig {
    pub orientation: ColorbarOrientation,
    /// Dimensions of the plot area (width, height) that the colorbar is attached to
    pub dimensions: [f32; 2],
    /// Width of the colorbar (thickness)
    pub colorbar_width: Option<f32>,
    /// Total height of the colorbar including background padding,
    /// if None will use plot height limited to 200px
    pub colorbar_height: Option<f32>,
    /// Margin between plot area and colorbar
    pub colorbar_margin: Option<f32>,
    /// Optional numeric formatting string for colorbar tick labels
    pub format_number: Option<String>,
    /// Optional background rect styling
    pub background_fill: Option<ColorOrGradient>,
    pub background_stroke: Option<ColorOrGradient>,
    pub background_corner_radius: Option<f32>,
    pub background_padding: Option<f32>,

    /// Typography configuration for axis title
    pub title_font_family: Option<String>,
    pub title_font_size: Option<f32>,
    pub title_font_weight: Option<FontWeight>,
    pub title_color: Option<[f32; 4]>,

    /// Typography configuration for axis labels
    pub label_font_family: Option<String>,
    pub label_font_size: Option<f32>,
    pub label_font_weight: Option<FontWeight>,
    pub label_color: Option<[f32; 4]>,

    /// Axis line and tick colors
    pub domain_color: Option<[f32; 4]>,
    pub tick_color: Option<[f32; 4]>,
}

impl Default for ColorbarConfig {
    fn default() -> Self {
        Self {
            orientation: ColorbarOrientation::Right,
            dimensions: [100.0, 100.0],
            colorbar_width: None,
            colorbar_height: None,
            colorbar_margin: None,
            format_number: None,
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
            title_font_family: None,
            title_font_size: None,
            title_font_weight: None,
            title_color: None,
            label_font_family: None,
            label_font_size: None,
            label_font_weight: None,
            label_color: None,
            domain_color: None,
            tick_color: None,
        }
    }
}
