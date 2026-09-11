use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array};
use avenger_color::{ColorOrGradient, Gradient, LinearGradient};
use avenger_format_number::NumberLocaleRegistry;
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark, rect::SceneRectMark};
use avenger_text::{
    default_text_engine,
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextSyntaxMode},
    LabelParams, NumberLocaleSpecs, TextEngine,
};

use crate::{
    axis::{
        numeric::{make_numeric_axis_marks_with_text_engine, make_tick_label_text},
        opts::{AxisConfig, AxisOrientation},
    },
    error::AvengerGuidesError,
    legend::{
        GuideLegendContinuousOrientation, GuideLegendContinuousSurface, GuideLegendOutput,
        GuideLegendSurfaceKind,
    },
};

pub fn make_colorbar_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &ColorbarConfig,
) -> Result<SceneGroup, AvengerGuidesError> {
    Ok(make_colorbar_marks_with_surfaces(scale, title, origin, config)?.group)
}

pub fn make_colorbar_marks_with_text_engine(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &ColorbarConfig,
    text_engine: &TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    Ok(make_colorbar_marks_with_surfaces_with_text_engine(
        scale,
        title,
        origin,
        config,
        text_engine,
    )?
    .group)
}

pub fn make_colorbar_marks_with_surfaces(
    scale: &ConfiguredScale,
    title: &str,
    _origin: [f32; 2], // Unused - we always start at (0, 0) now
    config: &ColorbarConfig,
) -> Result<GuideLegendOutput, AvengerGuidesError> {
    let text_engine = default_text_engine();
    make_colorbar_marks_with_surfaces_with_text_engine(scale, title, _origin, config, &text_engine)
}

fn colorbar_domain_label_text(
    scale: &ConfiguredScale,
    config: &ColorbarConfig,
) -> Result<(String, String, TextSyntaxMode), AvengerGuidesError> {
    let (domain_min, domain_max) = scale.config.numeric_interval_domain()?;
    let ticks = Arc::new(Float64Array::from(vec![
        domain_min as f64,
        domain_max as f64,
    ])) as ArrayRef;
    let axis_config = AxisConfig {
        format_number: config.format_number.clone(),
        format_datetime: None,
        tick_label: None,
        number_locale: config.number_locale.clone(),
        number_locale_registry: config.number_locale_registry.clone(),
        number_locale_specs: config.number_locale_specs.clone(),
        ..Default::default()
    };
    let tick_labels = make_tick_label_text(&ticks, scale, &axis_config)?;
    let labels = tick_labels.text.as_vec(2, None);
    Ok((
        labels.first().cloned().unwrap_or_default(),
        labels.get(1).cloned().unwrap_or_default(),
        tick_labels.syntax_mode,
    ))
}

pub fn make_colorbar_marks_with_surfaces_with_text_engine(
    scale: &ConfiguredScale,
    title: &str,
    _origin: [f32; 2], // Unused - we always start at (0, 0) now
    config: &ColorbarConfig,
    text_engine: &TextEngine,
) -> Result<GuideLegendOutput, AvengerGuidesError> {
    match config.orientation {
        ColorbarOrientation::Top => {
            // Horizontal gradient going left-to-right with axis above
            let available_width = config.dimensions[0];
            let _available_height = config.dimensions[1];

            let total_width = config.colorbar_height.unwrap_or(available_width.min(200.0));
            let bg_padding = config.background_padding.unwrap_or(4.0);

            // For horizontal colorbars (Top/Bottom), measure tick label widths to reserve horizontal space
            // This ensures tick labels at left/right don't run into the background edge
            let label_font_size = config.label_font_size.unwrap_or(10.0);
            let label_font_weight = config
                .label_font_weight
                .as_ref()
                .unwrap_or(&FontWeight::Number(400.0));
            let label_font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");

            let (min_label, max_label, label_syntax_mode) =
                colorbar_domain_label_text(scale, config)?;

            // Measure both labels and take the maximum width
            let min_bounds =
                text_engine.measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
                    text: &min_label,
                    font: label_font_family,
                    font_size: label_font_size,
                    font_weight: *label_font_weight,
                    font_style: FontStyle::Normal,
                    syntax_mode: label_syntax_mode,
                    params: avenger_text::empty_label_params(),
                    number_locale: config.number_locale.as_deref(),
                    number_locale_specs: Some(&config.number_locale_specs),
                    datetime_locale: None,
                    datetime_timezone: None,
                    datetime_locale_specs: None,
                });
            let max_bounds =
                text_engine.measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
                    text: &max_label,
                    font: label_font_family,
                    font_size: label_font_size,
                    font_weight: *label_font_weight,
                    font_style: FontStyle::Normal,
                    syntax_mode: label_syntax_mode,
                    params: avenger_text::empty_label_params(),
                    number_locale: config.number_locale.as_deref(),
                    number_locale_specs: Some(&config.number_locale_specs),
                    datetime_locale: None,
                    datetime_timezone: None,
                    datetime_locale_specs: None,
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
                format_datetime: None,
                tick_label: None,
                number_locale: config.number_locale.clone(),
                number_locale_registry: config.number_locale_registry.clone(),
                number_locale_specs: config.number_locale_specs.clone(),
                datetime_locale: None,
                datetime_timezone: None,
                datetime_locale_registry: None,
                datetime_locale_specs: avenger_text::DateTimeLocaleSpecs::default(),
                title_font_size: config.title_font_size,
                title_font_weight: config.title_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                title_font_family: config.title_font_family.clone(),
                title_color: config.title_color,
                title_syntax_mode: config.title_syntax_mode,
                title_text_params: config.title_text_params.clone(),
                label_font_size: config.label_font_size,
                label_font_weight: config.label_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                label_angle: None,
                label_font_family: config.label_font_family.clone(),
                label_color: config.label_color,
                domain_color: config.domain_color,
                tick_color: config.tick_color,
                grid_color: None,
                grid_width: None,
                tick_length: None,
                title_visible: Some(true),
                labels_visible: None,
                tick_count: Some(10.0),
                tick_start_step: None,
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
            let axis = noninteractive_group(make_numeric_axis_marks_with_text_engine(
                &numeric_scale,
                title,
                axis_origin,
                &axis_config,
                text_engine,
            )?);

            // Content marks
            let content_marks = vec![axis.clone().into(), rect.clone().into()];
            let content_group = SceneGroup {
                marks: content_marks.clone(),
                ..Default::default()
            };
            let content_bbox = content_group.bounding_box_with_text_engine(text_engine);

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
                interactive: false,
                ..Default::default()
            };

            // Shift content to account for any negative y overhang, then add bg_padding
            // Add horizontal_text_padding to ensure equal spacing for tick labels on left/right
            let axis_group_origin = [horizontal_text_padding + bg_padding, y_shift + bg_padding];
            let colorbar_axis_group = SceneGroup {
                origin: axis_group_origin,
                marks: vec![axis.into(), rect.into()],
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            };

            let marks = vec![bg_rect.into(), colorbar_axis_group.into()];

            Ok(colorbar_guide_output(
                SceneGroup {
                    origin: [0.0, 0.0],
                    marks,
                    clip: avenger_scenegraph::marks::group::Clip::None,
                    ..Default::default()
                },
                &config.orientation,
                axis_group_origin,
                1,
                gradient_width,
                colorbar_height,
            ))
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
            let label_font_size = config.label_font_size.unwrap_or(10.0);
            let label_font_weight = config
                .label_font_weight
                .as_ref()
                .unwrap_or(&FontWeight::Number(400.0));
            let label_font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");

            let (min_label, max_label, label_syntax_mode) =
                colorbar_domain_label_text(scale, config)?;

            // Measure both labels and take the maximum width
            let min_bounds =
                text_engine.measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
                    text: &min_label,
                    font: label_font_family,
                    font_size: label_font_size,
                    font_weight: *label_font_weight,
                    font_style: FontStyle::Normal,
                    syntax_mode: label_syntax_mode,
                    params: avenger_text::empty_label_params(),
                    number_locale: config.number_locale.as_deref(),
                    number_locale_specs: Some(&config.number_locale_specs),
                    datetime_locale: None,
                    datetime_timezone: None,
                    datetime_locale_specs: None,
                });
            let max_bounds =
                text_engine.measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
                    text: &max_label,
                    font: label_font_family,
                    font_size: label_font_size,
                    font_weight: *label_font_weight,
                    font_style: FontStyle::Normal,
                    syntax_mode: label_syntax_mode,
                    params: avenger_text::empty_label_params(),
                    number_locale: config.number_locale.as_deref(),
                    number_locale_specs: Some(&config.number_locale_specs),
                    datetime_locale: None,
                    datetime_timezone: None,
                    datetime_locale_specs: None,
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
                format_datetime: None,
                tick_label: None,
                number_locale: config.number_locale.clone(),
                number_locale_registry: config.number_locale_registry.clone(),
                number_locale_specs: config.number_locale_specs.clone(),
                datetime_locale: None,
                datetime_timezone: None,
                datetime_locale_registry: None,
                datetime_locale_specs: avenger_text::DateTimeLocaleSpecs::default(),
                title_font_size: config.title_font_size,
                title_font_weight: config.title_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                title_font_family: config.title_font_family.clone(),
                title_color: config.title_color,
                title_syntax_mode: config.title_syntax_mode,
                title_text_params: config.title_text_params.clone(),
                label_font_size: config.label_font_size,
                label_font_weight: config.label_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                label_angle: None,
                label_font_family: config.label_font_family.clone(),
                label_color: config.label_color,
                domain_color: config.domain_color,
                tick_color: config.tick_color,
                grid_color: None,
                grid_width: None,
                tick_length: None,
                title_visible: Some(true),
                labels_visible: None,
                tick_count: Some(10.0),
                tick_start_step: None,
            };

            // Create scale with horizontal range
            let numeric_scale = scale.clone().with_range_interval((0.0, gradient_width));

            // Position axis below the colorbar
            let axis_origin = [0.0, colorbar_height + colorbar_margin];
            let axis = noninteractive_group(make_numeric_axis_marks_with_text_engine(
                &numeric_scale,
                title,
                axis_origin,
                &axis_config,
                text_engine,
            )?);

            // Content marks
            let content_marks = vec![rect.clone().into(), axis.clone().into()];
            let content_group = SceneGroup {
                marks: content_marks.clone(),
                ..Default::default()
            };
            let content_bbox = content_group.bounding_box_with_text_engine(text_engine);

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
                interactive: false,
                ..Default::default()
            };

            // Shift content down if there's negative y overhang, then add bg_padding
            // Add horizontal_text_padding to ensure equal spacing for tick labels on left/right
            let axis_group_origin = [horizontal_text_padding + bg_padding, y_shift + bg_padding];
            let colorbar_axis_group = SceneGroup {
                origin: axis_group_origin,
                marks: vec![rect.into(), axis.into()],
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            };

            let marks = vec![bg_rect.into(), colorbar_axis_group.into()];

            Ok(colorbar_guide_output(
                SceneGroup {
                    origin: [0.0, 0.0],
                    marks,
                    clip: avenger_scenegraph::marks::group::Clip::None,
                    ..Default::default()
                },
                &config.orientation,
                axis_group_origin,
                0,
                gradient_width,
                colorbar_height,
            ))
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
            let label_font_size = config.label_font_size.unwrap_or(10.0);
            let label_font_weight = config
                .label_font_weight
                .as_ref()
                .unwrap_or(&FontWeight::Number(400.0));
            let label_font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");

            let text_bounds =
                text_engine.measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
                    text: "0",
                    font: label_font_family,
                    font_size: label_font_size,
                    font_weight: *label_font_weight,
                    font_style: FontStyle::Normal,
                    syntax_mode: avenger_text::types::TextSyntaxMode::Plain,
                    params: avenger_text::empty_label_params(),
                    number_locale: None,
                    number_locale_specs: None,
                    datetime_locale: None,
                    datetime_timezone: None,
                    datetime_locale_specs: None,
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
                format_datetime: None,
                tick_label: None,
                number_locale: config.number_locale.clone(),
                number_locale_registry: config.number_locale_registry.clone(),
                number_locale_specs: config.number_locale_specs.clone(),
                datetime_locale: None,
                datetime_timezone: None,
                datetime_locale_registry: None,
                datetime_locale_specs: avenger_text::DateTimeLocaleSpecs::default(),
                title_font_size: config.title_font_size,
                title_font_weight: config.title_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                title_font_family: config.title_font_family.clone(),
                title_color: config.title_color,
                title_syntax_mode: config.title_syntax_mode,
                title_text_params: config.title_text_params.clone(),
                label_font_size: config.label_font_size,
                label_font_weight: config.label_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                label_angle: None,
                label_font_family: config.label_font_family.clone(),
                label_color: config.label_color,
                domain_color: config.domain_color,
                tick_color: config.tick_color,
                grid_color: None,
                grid_width: None,
                tick_length: None,
                title_visible: Some(true),
                labels_visible: None,
                tick_count: None,
                tick_start_step: None,
            };

            let numeric_scale = scale.clone().with_range_interval((gradient_height, 0.0));

            // For Left axis: The axis renders to the LEFT of its origin point
            // We want: [axis content] [margin] [rect at 0]
            // So we need to find where to place the axis origin

            // First measure the axis to see its extent
            let _axis_temp = make_numeric_axis_marks_with_text_engine(
                &numeric_scale,
                title,
                [0.0, 0.0],
                &axis_config,
                text_engine,
            )?;

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
            let axis = noninteractive_group(make_numeric_axis_marks_with_text_engine(
                &numeric_scale,
                title,
                axis_origin,
                &axis_config,
                text_engine,
            )?);

            // Content marks
            let content_marks = vec![axis.clone().into(), rect.clone().into()];
            let content_group = SceneGroup {
                marks: content_marks.clone(),
                ..Default::default()
            };
            let content_bbox = content_group.bounding_box_with_text_engine(text_engine);

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
                interactive: false,
                ..Default::default()
            };

            // Shift content to account for any negative x overhang, then add bg_padding
            // Add vertical_text_padding to ensure equal spacing for tick labels
            let axis_group_origin = [x_shift + bg_padding, vertical_text_padding + bg_padding];
            let colorbar_axis_group = SceneGroup {
                origin: axis_group_origin,
                marks: vec![axis.into(), rect.into()],
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            };

            let marks = vec![bg_rect.into(), colorbar_axis_group.into()];

            Ok(colorbar_guide_output(
                SceneGroup {
                    origin: [0.0, 0.0],
                    marks,
                    clip: avenger_scenegraph::marks::group::Clip::None,
                    ..Default::default()
                },
                &config.orientation,
                axis_group_origin,
                1,
                colorbar_width,
                gradient_height,
            ))
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
            let label_font_size = config.label_font_size.unwrap_or(10.0);
            let label_font_weight = config
                .label_font_weight
                .as_ref()
                .unwrap_or(&FontWeight::Number(400.0));
            let label_font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");

            let text_bounds =
                text_engine.measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
                    text: "0",
                    font: label_font_family,
                    font_size: label_font_size,
                    font_weight: *label_font_weight,
                    font_style: FontStyle::Normal,
                    syntax_mode: avenger_text::types::TextSyntaxMode::Plain,
                    params: avenger_text::empty_label_params(),
                    number_locale: None,
                    number_locale_specs: None,
                    datetime_locale: None,
                    datetime_timezone: None,
                    datetime_locale_specs: None,
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
                format_datetime: None,
                tick_label: None,
                number_locale: config.number_locale.clone(),
                number_locale_registry: config.number_locale_registry.clone(),
                number_locale_specs: config.number_locale_specs.clone(),
                datetime_locale: None,
                datetime_timezone: None,
                datetime_locale_registry: None,
                datetime_locale_specs: avenger_text::DateTimeLocaleSpecs::default(),
                title_font_size: config.title_font_size,
                title_font_weight: config.title_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                title_font_family: config.title_font_family.clone(),
                title_color: config.title_color,
                title_syntax_mode: config.title_syntax_mode,
                title_text_params: config.title_text_params.clone(),
                label_font_size: config.label_font_size,
                label_font_weight: config.label_font_weight.as_ref().map(|w| match w {
                    FontWeight::Number(n) => *n,
                    FontWeight::Name(FontWeightNameSpec::Normal) => 400.0,
                    FontWeight::Name(FontWeightNameSpec::Bold) => 700.0,
                }),
                label_angle: None,
                label_font_family: config.label_font_family.clone(),
                label_color: config.label_color,
                domain_color: config.domain_color,
                tick_color: config.tick_color,
                grid_color: None,
                grid_width: None,
                tick_length: None,
                title_visible: Some(true),
                labels_visible: None,
                tick_count: None,
                tick_start_step: None,
            };

            // Create a new scale with desired range for the axis
            let numeric_scale = scale.clone().with_range_interval((gradient_height, 0.0));
            let axis = noninteractive_group(make_numeric_axis_marks_with_text_engine(
                &numeric_scale,
                title,
                axis_origin,
                &axis_config,
                text_engine,
            )?);

            // Content marks
            let content_marks = vec![rect.clone().into(), axis.clone().into()];
            let content_group = SceneGroup {
                marks: content_marks.clone(),
                ..Default::default()
            };
            let content_bbox = content_group.bounding_box_with_text_engine(text_engine);

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
                interactive: false,
                ..Default::default()
            };

            // Create a group for colorbar and axis with proper padding
            // Shift content right if there's negative x overhang, then add bg_padding
            // Add vertical_text_padding to ensure equal spacing for tick labels
            let axis_group_origin = [x_shift + bg_padding, vertical_text_padding + bg_padding];
            let colorbar_axis_group = SceneGroup {
                origin: axis_group_origin,
                marks: vec![rect.into(), axis.into()],
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            };

            // Insert background rect first, then legend content
            let marks = vec![bg_rect.into(), colorbar_axis_group.into()];

            Ok(colorbar_guide_output(
                SceneGroup {
                    origin: [0.0, 0.0],
                    marks,
                    clip: avenger_scenegraph::marks::group::Clip::None,
                    ..Default::default()
                },
                &config.orientation,
                axis_group_origin,
                0,
                colorbar_width,
                gradient_height,
            ))
        }
    }
}

fn noninteractive_group(mut group: SceneGroup) -> SceneGroup {
    group.interactive = false;
    for mark in &mut group.marks {
        set_interactive_recursive(mark, false);
    }
    group
}

fn set_interactive_recursive(mark: &mut SceneMark, interactive: bool) {
    mark.set_interactive(interactive);
    if let SceneMark::Group(group) = mark {
        for child in &mut group.marks {
            set_interactive_recursive(child, interactive);
        }
    }
}

fn colorbar_guide_output(
    group: SceneGroup,
    orientation: &ColorbarOrientation,
    axis_group_origin: [f32; 2],
    gradient_rect_index: usize,
    gradient_width: f32,
    gradient_height: f32,
) -> GuideLegendOutput {
    let (orientation, value_channel, band_channel) = match orientation {
        ColorbarOrientation::Top => (GuideLegendContinuousOrientation::Top, "x", "y"),
        ColorbarOrientation::Bottom => (GuideLegendContinuousOrientation::Bottom, "x", "y"),
        ColorbarOrientation::Left => (GuideLegendContinuousOrientation::Left, "y", "x"),
        ColorbarOrientation::Right => (GuideLegendContinuousOrientation::Right, "y", "x"),
    };
    let surface_group_path = vec![1];
    let gradient_rect_path = vec![1, gradient_rect_index];
    GuideLegendOutput {
        group,
        items: Vec::new(),
        continuous_surfaces: vec![GuideLegendContinuousSurface {
            kind: GuideLegendSurfaceKind::Colorbar,
            orientation,
            surface_group_path,
            hit_rect_path: gradient_rect_path.clone(),
            gradient_rect_path,
            bounds: [
                axis_group_origin[0],
                axis_group_origin[1],
                gradient_width,
                gradient_height,
            ],
            value_channel: value_channel.to_string(),
            band_channel: band_channel.to_string(),
        }],
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
    pub number_locale: Option<String>,
    pub number_locale_registry: Option<Arc<NumberLocaleRegistry>>,
    pub number_locale_specs: NumberLocaleSpecs,
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
    pub title_syntax_mode: TextSyntaxMode,
    pub title_text_params: LabelParams,

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
            number_locale: None,
            number_locale_registry: None,
            number_locale_specs: NumberLocaleSpecs::default(),
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
            title_font_family: None,
            title_font_size: None,
            title_font_weight: None,
            title_color: None,
            title_syntax_mode: TextSyntaxMode::Plain,
            title_text_params: LabelParams::default(),
            label_font_family: None,
            label_font_size: None,
            label_font_weight: None,
            label_color: None,
            domain_color: None,
            tick_color: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use avenger_geometry::rtree::{EnvelopeUtils, SceneGraphRTree};
    use avenger_scales::scales::linear::LinearScale;
    use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};

    use super::*;
    use crate::legend::{GuideLegendContinuousOrientation, GuideLegendSurfaceKind};

    fn color_scale() -> ConfiguredScale {
        LinearScale::configured_color((0.0, 100.0), ["#440154", "#fde725"])
    }

    fn test_config(orientation: ColorbarOrientation) -> ColorbarConfig {
        ColorbarConfig {
            orientation,
            dimensions: [240.0, 220.0],
            colorbar_width: Some(18.0),
            colorbar_height: Some(160.0),
            colorbar_margin: Some(4.0),
            background_padding: Some(4.0),
            ..Default::default()
        }
    }

    fn expected_orientation(orientation: &ColorbarOrientation) -> GuideLegendContinuousOrientation {
        match orientation {
            ColorbarOrientation::Top => GuideLegendContinuousOrientation::Top,
            ColorbarOrientation::Bottom => GuideLegendContinuousOrientation::Bottom,
            ColorbarOrientation::Left => GuideLegendContinuousOrientation::Left,
            ColorbarOrientation::Right => GuideLegendContinuousOrientation::Right,
        }
    }

    fn mark_at_path<'a>(marks: &'a [SceneMark], path: &[usize]) -> &'a SceneMark {
        let (first, rest) = path.split_first().expect("non-empty path");
        let mark = marks.get(*first).expect("mark at path segment");
        if rest.is_empty() {
            return mark;
        }
        let SceneMark::Group(group) = mark else {
            panic!("interior path segment should be a group");
        };
        mark_at_path(&group.marks, rest)
    }

    #[test]
    fn colorbar_domain_measurement_labels_use_shared_number_formatter() {
        let scale = LinearScale::configured_color((900_000.0, 1_100_000.0), ["#440154", "#fde725"]);
        let mut config = test_config(ColorbarOrientation::Top);
        config.format_number = Some("s".to_string());

        let (min_label, max_label, syntax_mode) =
            colorbar_domain_label_text(&scale, &config).expect("domain labels");

        assert_eq!(syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(min_label, "0.9M");
        assert_eq!(max_label, "1.1M");
    }

    #[test]
    fn colorbar_orientations_report_one_continuous_surface() {
        for orientation in [
            ColorbarOrientation::Top,
            ColorbarOrientation::Bottom,
            ColorbarOrientation::Left,
            ColorbarOrientation::Right,
        ] {
            let output = make_colorbar_marks_with_surfaces(
                &color_scale(),
                "temperature",
                [0.0, 0.0],
                &test_config(orientation.clone()),
            )
            .expect("colorbar renders");
            assert!(output.items.is_empty());
            assert_eq!(output.continuous_surfaces.len(), 1);

            let surface = &output.continuous_surfaces[0];
            assert_eq!(surface.kind, GuideLegendSurfaceKind::Colorbar);
            assert_eq!(surface.orientation, expected_orientation(&orientation));
            match orientation {
                ColorbarOrientation::Top | ColorbarOrientation::Bottom => {
                    assert_eq!(surface.value_channel, "x");
                    assert_eq!(surface.band_channel, "y");
                }
                ColorbarOrientation::Left | ColorbarOrientation::Right => {
                    assert_eq!(surface.value_channel, "y");
                    assert_eq!(surface.band_channel, "x");
                }
            }
        }
    }

    #[test]
    fn colorbar_surface_hit_path_exists_and_matches_gradient_rect_bounds() {
        for orientation in [
            ColorbarOrientation::Top,
            ColorbarOrientation::Bottom,
            ColorbarOrientation::Left,
            ColorbarOrientation::Right,
        ] {
            let output = make_colorbar_marks_with_surfaces(
                &color_scale(),
                "temperature",
                [0.0, 0.0],
                &test_config(orientation),
            )
            .expect("colorbar renders");
            let surface = &output.continuous_surfaces[0];
            assert_eq!(surface.hit_rect_path, surface.gradient_rect_path);

            let gradient_mark = mark_at_path(&output.group.marks, &surface.gradient_rect_path);
            assert!(gradient_mark.interactive());
            let SceneMark::Rect(rect) = gradient_mark else {
                panic!("colorbar hit path should resolve to the gradient rect");
            };
            let rect_bounds = rect.bounding_box();
            assert_eq!(rect_bounds.width(), surface.bounds[2]);
            assert_eq!(rect_bounds.height(), surface.bounds[3]);
        }
    }

    #[test]
    fn colorbar_rtree_hits_gradient_but_not_noninteractive_chrome() {
        let output = make_colorbar_marks_with_surfaces(
            &color_scale(),
            "temperature",
            [0.0, 0.0],
            &test_config(ColorbarOrientation::Right),
        )
        .expect("colorbar renders");
        let surface = &output.continuous_surfaces[0];
        let scene = SceneGraph {
            marks: output.group.marks.clone(),
            width: output.group.bounding_box().width(),
            height: output.group.bounding_box().height(),
            origin: output.group.origin,
        };
        let rtree = SceneGraphRTree::from_scene_graph(&scene);

        let gradient_point = [
            surface.bounds[0] + surface.bounds[2] * 0.5,
            surface.bounds[1] + surface.bounds[3] * 0.5,
        ];
        let hit = rtree
            .pick_top_mark_at_point(&gradient_point)
            .expect("gradient should be interactive");
        assert_eq!(hit.mark_path, surface.gradient_rect_path);

        let background_point = [1.0, 1.0];
        assert!(
            rtree.pick_top_mark_at_point(&background_point).is_none(),
            "transparent background/chrome should remain noninteractive"
        );
    }
}
