//! Avenger-language schemas and owner lowerers for facade-owned chart surface.

use avenger_chart_core::AxisGuideVisibilityPolicy;
use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, ResolvedDeclaration, ResolvedValue,
};
use avenger_chart_schema::{
    BodyMode, ChildRule, EnumValueSchema, KindSchema, NativeKindKey, NativeKindNamespace,
    PropertySchema, ValueShape,
};

use crate::concat::{GridConcat, HConcat, TrackSizing, VConcat, WrapConcat};

pub fn hconcat_definition() -> CoordinateLanguageDefinition<HConcat> {
    CoordinateLanguageDefinition::new("hconcat", hconcat_schema(), lower_hconcat)
}

pub fn vconcat_definition() -> CoordinateLanguageDefinition<VConcat> {
    CoordinateLanguageDefinition::new("vconcat", vconcat_schema(), lower_vconcat)
}

pub fn grid_concat_definition() -> CoordinateLanguageDefinition<GridConcat> {
    CoordinateLanguageDefinition::new("grid_concat", grid_concat_schema(), lower_grid_concat)
}

pub fn wrap_concat_definition() -> CoordinateLanguageDefinition<WrapConcat> {
    CoordinateLanguageDefinition::new("wrap_concat", wrap_concat_schema(), lower_wrap_concat)
}

fn container_schema(kind: &str, docs: &str) -> KindSchema {
    KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, kind),
        docs,
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "spacing",
        PropertySchema::optional(
            ValueShape::Number,
            "Minimum gap between adjacent plot areas.",
        ),
    )
    .child_rule(ChildRule {
        role: "cell".to_string(),
        min: 1,
        max: None,
        docs: "An ordered child plot cell.".to_string(),
    })
}

fn hconcat_schema() -> KindSchema {
    container_schema(
        "hconcat",
        "A horizontal ordered concatenation of child plot cells.",
    )
    .property(
        "widths",
        track_array("Per-cell horizontal plot-area track sizing."),
    )
}

fn vconcat_schema() -> KindSchema {
    container_schema(
        "vconcat",
        "A vertical ordered concatenation of child plot cells.",
    )
    .property(
        "heights",
        track_array("Per-cell vertical plot-area track sizing."),
    )
}

fn grid_concat_schema() -> KindSchema {
    container_schema(
        "grid_concat",
        "An explicitly placed two-dimensional grid of child plot cells.",
    )
    .property(
        "rows",
        PropertySchema::required(ValueShape::Integer, "Positive number of grid rows."),
    )
    .property(
        "columns",
        PropertySchema::required(ValueShape::Integer, "Positive number of grid columns."),
    )
    .property(
        "column_widths",
        track_array("Per-column plot-area track sizing."),
    )
    .property(
        "row_heights",
        track_array("Per-row plot-area track sizing."),
    )
    .property(
        "axis_guide_visibility",
        axis_visibility("Axis label and title compaction policy across grid cells."),
    )
}

fn wrap_concat_schema() -> KindSchema {
    container_schema(
        "wrap_concat",
        "A row-major wrapping concatenation of child plot cells.",
    )
    .property(
        "columns",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Expression yielding the fixed number of columns.",
        ),
    )
    .property(
        "responsive_columns",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Expression yielding the target minimum cell width in pixels.",
        ),
    )
    .property(
        "axis_guide_visibility",
        axis_visibility("Axis label and title compaction policy across wrapped cells."),
    )
}

fn track_array(docs: &str) -> PropertySchema {
    PropertySchema::optional(ValueShape::Array(Box::new(ValueShape::Any)), docs)
}

fn axis_visibility(docs: &str) -> PropertySchema {
    PropertySchema::optional(
        ValueShape::Atom {
            values: [
                ("auto", "Use the container's default compaction behavior."),
                ("all", "Show labels and titles on every child axis."),
                (
                    "outer_edges",
                    "Show labels and titles only on physical outer edges.",
                ),
                (
                    "outer_for_equivalent_domain_groups",
                    "Compact only for semantically equivalent aligned domains.",
                ),
            ]
            .into_iter()
            .map(|(value, docs)| EnumValueSchema {
                value: value.to_string(),
                docs: docs.to_string(),
            })
            .collect(),
        },
        docs,
    )
}

fn lower_hconcat(declaration: &ResolvedDeclaration) -> Result<HConcat, NativeLoweringError> {
    let mut coordinate = HConcat::new();
    for (name, value) in &declaration.properties {
        coordinate = match name.as_str() {
            "spacing" => coordinate.spacing(nonnegative_f32(name, value)?),
            "widths" => coordinate.widths(track_sizes(name, value)?),
            _ => return Err(invalid(name, "a registered hconcat property")),
        };
    }
    Ok(coordinate)
}

fn lower_vconcat(declaration: &ResolvedDeclaration) -> Result<VConcat, NativeLoweringError> {
    let mut coordinate = VConcat::new();
    for (name, value) in &declaration.properties {
        coordinate = match name.as_str() {
            "spacing" => coordinate.spacing(nonnegative_f32(name, value)?),
            "heights" => coordinate.heights(track_sizes(name, value)?),
            _ => return Err(invalid(name, "a registered vconcat property")),
        };
    }
    Ok(coordinate)
}

fn lower_grid_concat(declaration: &ResolvedDeclaration) -> Result<GridConcat, NativeLoweringError> {
    let mut coordinate = GridConcat::new();
    for (name, value) in &declaration.properties {
        coordinate = match name.as_str() {
            "spacing" => coordinate.spacing(nonnegative_f32(name, value)?),
            "rows" => coordinate.rows(positive_usize(name, value)?),
            "columns" => coordinate.columns(positive_usize(name, value)?),
            "column_widths" => coordinate.column_widths(track_sizes(name, value)?),
            "row_heights" => coordinate.row_heights(track_sizes(name, value)?),
            "axis_guide_visibility" => {
                coordinate.axis_guide_visibility(visibility_policy(name, value)?)
            }
            _ => return Err(invalid(name, "a registered grid_concat property")),
        };
    }
    Ok(coordinate)
}

fn lower_wrap_concat(declaration: &ResolvedDeclaration) -> Result<WrapConcat, NativeLoweringError> {
    if declaration.properties.contains_key("columns")
        && declaration.properties.contains_key("responsive_columns")
    {
        return Err(NativeLoweringError::Lowering {
            kind: "wrap_concat".to_string(),
            message: "columns and responsive_columns are mutually exclusive".to_string(),
        });
    }
    let mut coordinate = WrapConcat::new();
    for (name, value) in &declaration.properties {
        coordinate = match name.as_str() {
            "spacing" => coordinate.spacing(nonnegative_f32(name, value)?),
            "columns" => coordinate.columns(expr(name, value)?),
            "responsive_columns" => coordinate.responsive_columns(expr(name, value)?),
            "axis_guide_visibility" => {
                coordinate.axis_guide_visibility(visibility_policy(name, value)?)
            }
            _ => return Err(invalid(name, "a registered wrap_concat property")),
        };
    }
    Ok(coordinate)
}

fn track_sizes(
    property: &str,
    value: &ResolvedValue,
) -> Result<Vec<TrackSizing>, NativeLoweringError> {
    let ResolvedValue::Array(values) = value else {
        return Err(invalid(
            property,
            "an array of auto, px(number), or fr(number)",
        ));
    };
    values
        .iter()
        .map(|value| match value {
            ResolvedValue::String(value) if value == "auto" => Ok(TrackSizing::Auto),
            ResolvedValue::Call { function, args }
                if matches!(function.as_str(), "px" | "fr") && args.len() == 1 =>
            {
                let value = nonnegative_f32(property, &args[0])?;
                if function == "px" {
                    Ok(TrackSizing::Px(value))
                } else if value > 0.0 {
                    Ok(TrackSizing::Flex(value))
                } else {
                    Err(invalid(property, "fr(...) with a positive weight"))
                }
            }
            _ => Err(invalid(
                property,
                "an array of auto, px(number), or fr(number)",
            )),
        })
        .collect()
}

fn visibility_policy(
    property: &str,
    value: &ResolvedValue,
) -> Result<AxisGuideVisibilityPolicy, NativeLoweringError> {
    let ResolvedValue::String(value) = value else {
        return Err(invalid(property, "an axis-guide visibility policy"));
    };
    match value.as_str() {
        "auto" => Ok(AxisGuideVisibilityPolicy::Auto),
        "all" => Ok(AxisGuideVisibilityPolicy::All),
        "outer_edges" => Ok(AxisGuideVisibilityPolicy::OuterEdges),
        "outer_for_equivalent_domain_groups" => {
            Ok(AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups)
        }
        _ => Err(invalid(property, "an axis-guide visibility policy")),
    }
}

fn expr(
    property: &str,
    value: &ResolvedValue,
) -> Result<datafusion::logical_expr::Expr, NativeLoweringError> {
    avenger_chart_lang_types::resolved_expr(value)
        .ok_or_else(|| invalid(property, "a scalar SQL expression"))
}

fn nonnegative_f32(property: &str, value: &ResolvedValue) -> Result<f32, NativeLoweringError> {
    let value = match value {
        ResolvedValue::Integer(value) => *value as f64,
        ResolvedValue::Number(value) => *value,
        _ => return Err(invalid(property, "a finite non-negative number")),
    };
    if value.is_finite() && (0.0..=f32::MAX as f64).contains(&value) {
        Ok(value as f32)
    } else {
        Err(invalid(property, "a finite non-negative number"))
    }
}

fn positive_usize(property: &str, value: &ResolvedValue) -> Result<usize, NativeLoweringError> {
    let ResolvedValue::Integer(value) = value else {
        return Err(invalid(property, "a positive integer"));
    };
    usize::try_from(*value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| invalid(property, "a positive integer"))
}

fn invalid(property: &str, expected: &str) -> NativeLoweringError {
    NativeLoweringError::InvalidPropertyType {
        property: property.to_string(),
        expected: expected.to_string(),
    }
}
