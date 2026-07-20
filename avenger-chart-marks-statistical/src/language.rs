//! Avenger-language schemas and erased lowerers for statistical compound marks.

use avenger_chart_cartesian::Cartesian;
use avenger_chart_core::{
    ChannelExpr, ChannelValue, CoordinationScope, FacetDataScope, IntoPlotMark, PlotMark,
};
use avenger_chart_lang_types::{
    MarkLanguageDefinition, MarkLanguageLowerer, NativeLoweringError, NativeOutputValue,
    ResolvedDeclaration, ResolvedValue,
};
use avenger_chart_schema::{
    ChannelSchema, EnumValueSchema, KindSchema, NativeKindKey, PartSchema, PropertySchema,
    ValueShape,
};
use avenger_chart_transforms::KdeResolve;

use crate::{BoxPlot, BoxPlotOrientation, Violin, ViolinOrientation, ViolinWidthNormalization};

/// Complete stock statistical-mark inventory for Cartesian plots.
pub fn definitions() -> Vec<MarkLanguageDefinition<Cartesian>> {
    vec![
        MarkLanguageDefinition {
            kind: "box_plot",
            schema: box_plot_schema(),
            lowerer: MarkLanguageLowerer::Declaration(lower_box_plot),
        },
        MarkLanguageDefinition {
            kind: "violin",
            schema: violin_schema(),
            lowerer: MarkLanguageLowerer::Declaration(lower_violin),
        },
    ]
}

fn statistical_schema(kind: &str, docs: &str) -> KindSchema {
    KindSchema::new(NativeKindKey::mark("cartesian", kind), docs)
        .channel(channel(
            "x",
            true,
            "Horizontal position or grouping encoding.",
        ))
        .channel(channel(
            "y",
            true,
            "Vertical position or grouping encoding.",
        ))
        .property(
            "orientation",
            PropertySchema::optional(
                atom(&[
                    ("horizontal", "Use x for values and y for groups."),
                    ("vertical", "Use y for values and x for groups."),
                ]),
                "Explicit value-axis orientation; otherwise inferred from the position channels.",
            ),
        )
        .property(
            "facet_data_scope",
            PropertySchema::optional(
                ValueShape::FacetDataScope,
                "Facet visibility scope: filtered, broadcast, or level(n).",
            ),
        )
}

fn box_plot_schema() -> KindSchema {
    let mut schema = statistical_schema(
        "box_plot",
        "A compound box-and-whisker plot generated from Cartesian primitives and transforms.",
    )
    .channel(channel(
        "fill",
        false,
        "Shared categorical fill encoding for generated summary parts.",
    ))
    .property(
        "extent",
        PropertySchema::optional(
            ValueShape::Number,
            "Non-negative interquartile-range multiplier used for whisker fences; defaults to 1.5.",
        ),
    );
    for (alias, runtime_kind, docs) in [
        ("box", "rect", "The interquartile box body."),
        ("median", "rule", "The median rule."),
        ("whiskers", "rule", "The lower and upper whisker rules."),
        ("lower_cap", "rule", "The lower whisker cap."),
        ("upper_cap", "rule", "The upper whisker cap."),
        (
            "outliers",
            "symbol",
            "Raw observations outside the whisker fences.",
        ),
    ] {
        schema = schema.part(PartSchema {
            alias: alias.to_string(),
            runtime_kind: runtime_kind.to_string(),
            runtime_alias: Some(alias.to_string()),
            targetable: true,
            docs: docs.to_string(),
        });
    }
    schema
}

fn violin_schema() -> KindSchema {
    statistical_schema(
        "violin",
        "A compound kernel-density violin generated from Cartesian primitives and transforms.",
    )
    .channel(channel("fill", false, "Violin body fill encoding."))
    .channel(channel("stroke", false, "Violin body stroke encoding."))
    .channel(channel(
        "stroke_width",
        false,
        "Violin body stroke-width encoding.",
    ))
    .channel(channel(
        "stroke_dash",
        false,
        "Violin body stroke-dash encoding.",
    ))
    .channel(channel("opacity", false, "Violin body opacity encoding."))
    .property(
        "bandwidth",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Kernel bandwidth expression; zero requests automatic bandwidth selection.",
        ),
    )
    .property(
        "steps",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Number of density samples as a scalar SQL expression; defaults to 200.",
        ),
    )
    .property(
        "density_extent",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::SqlExpression)),
            "Two scalar SQL expressions defining the density sample interval.",
        ),
    )
    .property(
        "counts",
        PropertySchema::optional(
            ValueShape::Boolean,
            "Scale density values by the number of samples in each group.",
        ),
    )
    .property(
        "density_extent_resolve",
        PropertySchema::optional(
            atom(&[
                ("independent", "Infer a separate extent for each violin."),
                ("shared", "Infer one extent shared by all violins."),
            ]),
            "Coordination strategy for inferred KDE extents.",
        ),
    )
    .property(
        "density_data_scope",
        PropertySchema::optional(
            ValueShape::CoordinationScope,
            "Facet coordination scope used to compute density values.",
        ),
    )
    .property(
        "width",
        PropertySchema::optional(
            ValueShape::Number,
            "Fraction of the group band occupied by the violin, in (0, 1].",
        ),
    )
    .property(
        "width_normalization",
        PropertySchema::optional(
            atom(&[
                (
                    "shared",
                    "Normalize all violin widths by one maximum density.",
                ),
                (
                    "per_violin",
                    "Normalize each violin by its own maximum density.",
                ),
            ]),
            "Density-to-width normalization strategy.",
        ),
    )
    .part(PartSchema {
        alias: "body".to_string(),
        runtime_kind: "area".to_string(),
        runtime_alias: Some("body".to_string()),
        targetable: true,
        docs: "The generated density area body.".to_string(),
    })
}

fn channel(name: &str, required: bool, docs: &str) -> ChannelSchema {
    ChannelSchema {
        name: name.to_string(),
        required,
        shape: ValueShape::SqlExpression,
        docs: docs.to_string(),
    }
}

fn atom(values: &[(&str, &str)]) -> ValueShape {
    ValueShape::Atom {
        values: values
            .iter()
            .map(|(value, docs)| EnumValueSchema {
                value: (*value).to_string(),
                docs: (*docs).to_string(),
            })
            .collect(),
    }
}

fn lower_box_plot(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<PlotMark<Cartesian>>, NativeLoweringError> {
    let mut mark = BoxPlot::new();
    if let Some(name) = &declaration.source_name {
        mark = mark.id(name.clone());
    }
    mark = mark
        .x(channel_expr("x", declaration.get("x")?)?)
        .y(channel_expr("y", declaration.get("y")?)?);
    for (name, value) in &declaration.properties {
        mark = match name.as_str() {
            "x" | "y" => mark,
            "fill" => mark.fill(channel_value(name, value)?),
            "orientation" => mark.orientation(match string(name, value)? {
                "horizontal" => BoxPlotOrientation::Horizontal,
                "vertical" => BoxPlotOrientation::Vertical,
                _ => return Err(invalid(name, "horizontal or vertical")),
            }),
            "extent" => mark.extent(number(name, value)?),
            "facet_data_scope" => mark.facet_data_scope(facet_scope(name, value)?),
            _ => return Err(invalid(name, "a registered box_plot property")),
        };
    }
    Ok(mark.into_plot_marks())
}

fn lower_violin(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<PlotMark<Cartesian>>, NativeLoweringError> {
    let mut mark = Violin::new();
    if let Some(name) = &declaration.source_name {
        mark = mark.id(name.clone());
    }
    mark = mark
        .x(channel_expr("x", declaration.get("x")?)?)
        .y(channel_expr("y", declaration.get("y")?)?);
    for (name, value) in &declaration.properties {
        mark = match name.as_str() {
            "x" | "y" => mark,
            "fill" => mark.fill(channel_value(name, value)?),
            "stroke" => mark.stroke(channel_value(name, value)?),
            "stroke_width" => mark.stroke_width(channel_value(name, value)?),
            "stroke_dash" => mark.stroke_dash(channel_value(name, value)?),
            "opacity" => mark.opacity(channel_value(name, value)?),
            "orientation" => mark.orientation(match string(name, value)? {
                "horizontal" => ViolinOrientation::Horizontal,
                "vertical" => ViolinOrientation::Vertical,
                _ => return Err(invalid(name, "horizontal or vertical")),
            }),
            "bandwidth" => mark.bandwidth(expression(name, value)?),
            "steps" => mark.steps(expression(name, value)?),
            "density_extent" => {
                let ResolvedValue::Array(values) = value else {
                    return Err(invalid(name, "two SQL expressions"));
                };
                let [start, stop] = values.as_slice() else {
                    return Err(invalid(name, "exactly two SQL expressions"));
                };
                mark.density_extent(expression(name, start)?, expression(name, stop)?)
            }
            "counts" => {
                let ResolvedValue::Boolean(value) = value else {
                    return Err(invalid(name, "boolean"));
                };
                mark.counts(*value)
            }
            "density_extent_resolve" => mark.density_extent_resolve(match string(name, value)? {
                "independent" => KdeResolve::Independent,
                "shared" => KdeResolve::Shared,
                _ => return Err(invalid(name, "independent or shared")),
            }),
            "density_data_scope" => mark.density_data_scope(coordination_scope(name, value)?),
            "width" => mark.width(number(name, value)?),
            "width_normalization" => mark.width_normalization(match string(name, value)? {
                "shared" => ViolinWidthNormalization::Shared,
                "per_violin" => ViolinWidthNormalization::PerViolin,
                _ => return Err(invalid(name, "shared or per_violin")),
            }),
            "facet_data_scope" => mark.facet_data_scope(facet_scope(name, value)?),
            _ => return Err(invalid(name, "a registered violin property")),
        };
    }
    Ok(mark.into_plot_marks())
}

fn channel_expr(property: &str, value: &ResolvedValue) -> Result<ChannelExpr, NativeLoweringError> {
    match value {
        ResolvedValue::Channel(value) => Ok(value.as_ref().clone()),
        ResolvedValue::Expr(expr) => Ok(ChannelExpr::scaled(expr.clone())),
        ResolvedValue::Output(NativeOutputValue::Expr(expr)) => {
            Ok(ChannelExpr::scaled(expr.clone()))
        }
        ResolvedValue::Output(NativeOutputValue::Channel(value)) => Ok(value.clone()),
        _ => Err(invalid(property, "resolved channel expression")),
    }
}

fn channel_value(
    property: &str,
    value: &ResolvedValue,
) -> Result<ChannelValue, NativeLoweringError> {
    match value {
        ResolvedValue::Expr(expr) => Ok(expr.clone().into()),
        ResolvedValue::Channel(value) => Ok(value.channel_value().clone()),
        ResolvedValue::Output(NativeOutputValue::Expr(expr)) => Ok(expr.clone().into()),
        ResolvedValue::Output(NativeOutputValue::Channel(value)) => {
            Ok(value.channel_value().clone())
        }
        _ => Err(invalid(property, "resolved channel value")),
    }
}

fn expression(
    property: &str,
    value: &ResolvedValue,
) -> Result<datafusion::logical_expr::Expr, NativeLoweringError> {
    match value {
        ResolvedValue::Expr(expr) => Ok(expr.clone()),
        ResolvedValue::Output(NativeOutputValue::Expr(expr)) => Ok(expr.clone()),
        _ => Err(invalid(property, "scalar SQL expression")),
    }
}

fn string<'a>(property: &str, value: &'a ResolvedValue) -> Result<&'a str, NativeLoweringError> {
    let ResolvedValue::String(value) = value else {
        return Err(invalid(property, "enum atom"));
    };
    Ok(value)
}

fn number(property: &str, value: &ResolvedValue) -> Result<f64, NativeLoweringError> {
    match value {
        ResolvedValue::Number(value) => Ok(*value),
        ResolvedValue::Integer(value) => Ok(*value as f64),
        _ => Err(invalid(property, "number")),
    }
}

fn facet_scope(
    property: &str,
    value: &ResolvedValue,
) -> Result<FacetDataScope, NativeLoweringError> {
    let ResolvedValue::Integer(level) = value else {
        return Err(invalid(property, "resolved facet scope level"));
    };
    let level = (*level)
        .try_into()
        .map_err(|_| invalid(property, "scope level from 0 through 255"))?;
    Ok(FacetDataScope::level(level))
}

fn coordination_scope(
    property: &str,
    value: &ResolvedValue,
) -> Result<CoordinationScope, NativeLoweringError> {
    match value {
        ResolvedValue::String(value) if value == "shared" => Ok(CoordinationScope::Shared),
        ResolvedValue::String(value) if value == "free" => Ok(CoordinationScope::Free),
        ResolvedValue::Integer(level) => (*level)
            .try_into()
            .map(CoordinationScope::Level)
            .map_err(|_| invalid(property, "shared, free, or level from 0 through 255")),
        _ => Err(invalid(property, "shared, free, or a resolved level")),
    }
}

fn invalid(property: &str, expected: &str) -> NativeLoweringError {
    NativeLoweringError::InvalidPropertyType {
        property: property.to_string(),
        expected: expected.to_string(),
    }
}
