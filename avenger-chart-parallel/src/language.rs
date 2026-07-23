//! Avenger-language registration for parallel coordinates.

use std::collections::BTreeMap;

use avenger_chart_core::{ChannelValue, IntoPlotMark};
use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, ResolvedDeclaration, ResolvedValue,
    resolved_expr,
};
use avenger_chart_marks::language::{
    apply_common_mark_state, channel, is_common_mark_property, primitive_schema,
};
use avenger_chart_schema::{
    BodyMode, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema, ValueShape,
};

use crate::{Parallel, ParallelAxis, ParallelLine, ParallelSymbol, generated_dimension_channel};

const LINE_CHANNELS: &[&str] = &[
    "stroke",
    "stroke_width",
    "stroke_dash",
    "opacity",
    "stroke_cap",
    "stroke_join",
    "defined",
];
const SYMBOL_CHANNELS: &[&str] = &[
    "size",
    "fill",
    "stroke",
    "stroke_width",
    "shape",
    "angle",
    "opacity",
];

pub fn definition() -> CoordinateLanguageDefinition<Parallel> {
    CoordinateLanguageDefinition::new("parallel", coordinate_schema(), lower_parallel)
        .mark(
            "parallel_line",
            parallel_mark_schema(
                "parallel_line",
                "A wide-form polyline spanning the declared parallel dimensions.",
                LINE_CHANNELS,
            ),
            lower_parallel_line,
        )
        .mark(
            "parallel_symbol",
            parallel_mark_schema(
                "parallel_symbol",
                "A symbol at every row and parallel-dimension intersection.",
                SYMBOL_CHANNELS,
            ),
            lower_parallel_symbol,
        )
}

fn coordinate_schema() -> KindSchema {
    KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, "parallel"),
        "A wide-form parallel-coordinate frame with user-named dimensions.",
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "dimensions",
        PropertySchema::optional(
            ValueShape::Map(Box::new(ValueShape::Object(BTreeMap::from([(
                "axis".to_string(),
                PropertySchema::optional(
                    ValueShape::Object(parallel_axis_properties()),
                    "Shared axis configuration for this logical dimension.",
                ),
            )])))),
            "Sparse frame configuration keyed by mark-owned logical dimension id.",
        ),
    )
    .property(
        "order",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::Identifier)),
            "Static left-to-right dimension order; undeclared dimensions follow declaration order.",
        ),
    )
}

fn parallel_axis_properties() -> BTreeMap<String, PropertySchema> {
    [
        ("visible", "Whether the axis is visible."),
        ("title", "Axis title text."),
        ("grid", "Whether to draw grid lines."),
        ("tick_count", "Requested number of ticks."),
        (
            "tick_spacing",
            "Structured start/step tick-spacing expression.",
        ),
        ("label_angle", "Tick-label rotation angle in degrees."),
        ("format", "Number-format pattern."),
        ("title_font_family", "Axis-title font family."),
        ("title_color", "Axis-title color."),
        ("label_font_family", "Tick-label font family."),
        (
            "show_title",
            "Whether to render the title while retaining its configuration.",
        ),
    ]
    .into_iter()
    .map(|(name, docs)| {
        (
            name.to_string(),
            PropertySchema::optional(ValueShape::SqlExpression, docs),
        )
    })
    .collect()
}

fn parallel_mark_schema(kind: &str, docs: &str, channels: &[&'static str]) -> KindSchema {
    primitive_schema(
        "parallel",
        kind,
        docs,
        channels.iter().copied().map(channel),
    )
    .property(
        "dimensions",
        PropertySchema::required(
            ValueShape::ChannelMap,
            "User-named dimension ids mapped to configured encoding channels.",
        ),
    )
}

fn lower_parallel(declaration: &ResolvedDeclaration) -> Result<Parallel, NativeLoweringError> {
    let mut coordinate = Parallel::new();
    if let Some(ResolvedValue::Object(dimensions)) = declaration.properties.get("dimensions") {
        for (id, value) in dimensions {
            let ResolvedValue::Object(frame) = value else {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: format!("dimensions.{id}"),
                    expected: "parallel frame configuration object".to_string(),
                });
            };
            if let Some(axis) = frame.get("axis") {
                let ResolvedValue::Object(axis) = axis else {
                    return Err(NativeLoweringError::InvalidPropertyType {
                        property: format!("dimensions.{id}.axis"),
                        expected: "parallel axis configuration object".to_string(),
                    });
                };
                let axis = lower_parallel_axis(axis)?;
                coordinate =
                    coordinate.dimension_with(id.clone(), move |config| config.axis(|_| axis));
            } else {
                coordinate = coordinate.dimension(id.clone());
            }
        }
    }
    if let Some(ResolvedValue::Array(order)) = declaration.properties.get("order") {
        let ids = order
            .iter()
            .map(|value| match value {
                ResolvedValue::String(value) => Ok(value.clone()),
                _ => Err(NativeLoweringError::InvalidPropertyType {
                    property: "order".to_string(),
                    expected: "identifier array".to_string(),
                }),
            })
            .collect::<Result<Vec<_>, _>>()?;
        coordinate = coordinate.order(ids);
    }
    Ok(coordinate)
}

fn lower_parallel_axis(
    properties: &indexmap::IndexMap<String, ResolvedValue>,
) -> Result<ParallelAxis, NativeLoweringError> {
    let mut axis = ParallelAxis::new();
    for (name, value) in properties {
        let Some(expr) = resolved_expr(value) else {
            return Err(NativeLoweringError::InvalidPropertyType {
                property: format!("dimensions.*.axis.{name}"),
                expected: "scalar SQL expression".to_string(),
            });
        };
        axis = match name.as_str() {
            "visible" => axis.visible(expr),
            "title" => axis.title(expr),
            "grid" => axis.grid(expr),
            "tick_count" => axis.tick_count(expr),
            "tick_spacing" => axis.tick_spacing(expr),
            "label_angle" => axis.label_angle(expr),
            "format" => axis.format(expr),
            "title_font_family" => axis.title_font_family(expr),
            "title_color" => axis.title_color(expr),
            "label_font_family" => axis.label_font_family(expr),
            "show_title" => axis.show_title(expr),
            _ => {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: format!("dimensions.*.axis.{name}"),
                    expected: "registered parallel axis property".to_string(),
                });
            }
        };
    }
    Ok(axis)
}

fn dimension_channels(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<(String, ChannelValue)>, NativeLoweringError> {
    let ResolvedValue::Object(dimensions) = declaration.get("dimensions")? else {
        return Err(NativeLoweringError::InvalidPropertyType {
            property: "dimensions".to_string(),
            expected: "configured channel map".to_string(),
        });
    };
    dimensions
        .iter()
        .map(|(id, value)| {
            let ResolvedValue::Channel(value) = value else {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: format!("dimensions.{id}"),
                    expected: "resolved channel value".to_string(),
                });
            };
            Ok((
                id.clone(),
                value.channel_value().clone().with_scale_name(id),
            ))
        })
        .collect()
}

fn ordinary_channel(
    name: &str,
    value: &ResolvedValue,
) -> Result<ChannelValue, NativeLoweringError> {
    match value {
        ResolvedValue::Channel(value) => Ok(value.channel_value().clone()),
        ResolvedValue::Expr(value) => Ok(value.clone().into()),
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: name.to_string(),
            expected: "resolved channel value".to_string(),
        }),
    }
}

fn lower_parallel_line(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<avenger_chart_core::PlotMark<Parallel>>, NativeLoweringError> {
    let mut mark = ParallelLine::new();
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }
    for (id, value) in dimension_channels(declaration)? {
        mark = mark.with_channel_value(&generated_dimension_channel(&id), value);
    }
    for (name, value) in &declaration.properties {
        if name != "dimensions" && !is_common_mark_property(name) {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
    apply_common_mark_state::<Parallel, _>(&mut mark, declaration)?;
    Ok(mark.into_plot_marks())
}

fn lower_parallel_symbol(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<avenger_chart_core::PlotMark<Parallel>>, NativeLoweringError> {
    let mut mark = ParallelSymbol::new();
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }
    for (id, value) in dimension_channels(declaration)? {
        mark = mark.with_channel_value(&generated_dimension_channel(&id), value);
    }
    for (name, value) in &declaration.properties {
        if name != "dimensions" && !is_common_mark_property(name) {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
    apply_common_mark_state::<Parallel, _>(&mut mark, declaration)?;
    Ok(mark.into_plot_marks())
}
