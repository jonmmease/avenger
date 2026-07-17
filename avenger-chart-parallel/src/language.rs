//! Avenger-language registration for parallel coordinates.

use avenger_chart_core::{ChannelValue, IntoPlotMark};
use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, ResolvedDeclaration, ResolvedValue,
};
use avenger_chart_marks::language::{channel, primitive_schema};
use avenger_chart_schema::{
    BodyMode, ChildRule, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema, ValueShape,
};

use crate::{Parallel, ParallelLine, ParallelSymbol, generated_dimension_channel};

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
        "order",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::Identifier)),
            "Static left-to-right dimension order; undeclared dimensions follow declaration order.",
        ),
    )
    .child_rule(ChildRule {
        role: "dimension".to_string(),
        min: 1,
        max: None,
        docs: "A stable frame dimension id with optional axis configuration.".to_string(),
    })
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
    for dimension in &declaration.children {
        if dimension.kind == "dimension" {
            let id =
                dimension
                    .source_name
                    .as_ref()
                    .ok_or_else(|| NativeLoweringError::Lowering {
                        kind: declaration.kind.clone(),
                        message: "parallel dimension is missing its stable id".to_string(),
                    })?;
            coordinate = coordinate.dimension(id.clone());
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
            Ok((id.clone(), value.as_ref().clone().with_scale_name(id)))
        })
        .collect()
}

fn ordinary_channel(
    name: &str,
    value: &ResolvedValue,
) -> Result<ChannelValue, NativeLoweringError> {
    match value {
        ResolvedValue::Channel(value) => Ok(value.as_ref().clone()),
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
        if name != "dimensions" {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
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
        if name != "dimensions" {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
    Ok(mark.into_plot_marks())
}
