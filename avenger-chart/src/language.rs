//! Avenger-language schemas and owner lowerers for facade-owned chart surface.

use std::time::Duration;

use avenger_chart_core::AxisGuideVisibilityPolicy;
use avenger_chart_core::{
    CoordinationScope, RepeatDomainCoordination, RepeatTypeHint, RepeatVariable, View, ViewSpec,
    ViewStalePolicy,
};
use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, ObjectLanguageDefinition,
    ResolvedDeclaration, ResolvedValue,
};
use avenger_chart_schema::{
    BodyMode, ChildRule, EnumValueSchema, KindSchema, NativeKindKey, NativeKindNamespace,
    PropertySchema, ValueShape,
};

use crate::{
    concat::{GridConcat, HConcat, TrackSizing, VConcat, WrapConcat},
    facet::coord::{FacetColumn, FacetRow, FacetWrap},
    repeat::{RepeatColumns, RepeatGrid, RepeatRows, RepeatWrap},
};

pub fn cartesian_view_definition() -> ObjectLanguageDefinition {
    ObjectLanguageDefinition {
        schema: view_schema("cartesian", true),
        lowerer: lower_cartesian_view,
    }
}

pub fn pixel_frame_view_definition() -> ObjectLanguageDefinition {
    ObjectLanguageDefinition {
        schema: view_schema("pixel_frame", false),
        lowerer: lower_pixel_frame_view,
    }
}

fn view_schema(kind: &str, domains: bool) -> KindSchema {
    let mut schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::View, kind),
        if domains {
            "A Cartesian inline view whose domains drive view-local transforms."
        } else {
            "A scale-free logical-pixel inline view."
        },
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "stale_policy",
        PropertySchema::optional(
            atom(&[
                (
                    "hide_until_ready",
                    "Hide results until the current evaluation is ready.",
                ),
                (
                    "retarget_cached",
                    "Retarget the last ready result through current scales.",
                ),
            ]),
            "Behavior while a newer view-local result is pending.",
        ),
    )
    .property(
        "throttle_ms",
        PropertySchema::optional(
            ValueShape::Integer,
            "Minimum interval between evaluations in milliseconds.",
        ),
    )
    .property(
        "debounce_ms",
        PropertySchema::optional(
            ValueShape::Integer,
            "Quiet interval before evaluation in milliseconds.",
        ),
    )
    .child_rule(ChildRule {
        role: "transform".to_string(),
        min: 0,
        max: None,
        docs: "A view-local transform stage.".to_string(),
    })
    .child_rule(ChildRule {
        role: "mark".to_string(),
        min: 0,
        max: None,
        docs: "A primitive mark rendered from the view-local relation.".to_string(),
    })
    .child_rule(ChildRule {
        role: "group".to_string(),
        min: 0,
        max: None,
        docs: "A group rendered from the view-local relation.".to_string(),
    });
    if domains {
        schema = schema
            .property(
                "x_domain",
                PropertySchema::required(
                    ValueShape::SqlExpression,
                    "Expression whose values define the horizontal view domain.",
                ),
            )
            .property(
                "y_domain",
                PropertySchema::required(
                    ValueShape::SqlExpression,
                    "Expression whose values define the vertical view domain.",
                ),
            );
    }
    schema
}

fn lower_cartesian_view(
    declaration: &ResolvedDeclaration,
) -> Result<Box<dyn std::any::Any + Send + Sync>, NativeLoweringError> {
    let id = declaration
        .source_name
        .as_ref()
        .ok_or_else(|| NativeLoweringError::Lowering {
            kind: declaration.kind.clone(),
            message: "inline view requires a compiler-assigned identity".to_string(),
        })?;
    let view = View::cartesian()
        .id(id)
        .x_domain(expr("x_domain", declaration.get("x_domain")?)?)
        .y_domain(expr("y_domain", declaration.get("y_domain")?)?);
    let view = apply_cartesian_view_policy(declaration, view)?;
    let (compiled, _) = view.into_compiled_and_ref()?;
    Ok(Box::new(compiled))
}

fn lower_pixel_frame_view(
    declaration: &ResolvedDeclaration,
) -> Result<Box<dyn std::any::Any + Send + Sync>, NativeLoweringError> {
    let id = declaration
        .source_name
        .as_ref()
        .ok_or_else(|| NativeLoweringError::Lowering {
            kind: declaration.kind.clone(),
            message: "inline view requires a compiler-assigned identity".to_string(),
        })?;
    let mut view = View::pixel_frame().id(id);
    if let Some(ResolvedValue::String(policy)) = declaration.properties.get("stale_policy") {
        view = view.stale_policy(parse_stale_policy(policy)?);
    }
    if let Some(duration) = view_duration(declaration, "throttle_ms")? {
        view = view.throttle(duration);
    }
    if let Some(duration) = view_duration(declaration, "debounce_ms")? {
        view = view.debounce(duration);
    }
    let (compiled, _) = view.into_compiled_and_ref()?;
    Ok(Box::new(compiled))
}

fn apply_cartesian_view_policy(
    declaration: &ResolvedDeclaration,
    mut view: avenger_chart_core::CartesianView,
) -> Result<avenger_chart_core::CartesianView, NativeLoweringError> {
    if let Some(ResolvedValue::String(policy)) = declaration.properties.get("stale_policy") {
        view = view.stale_policy(parse_stale_policy(policy)?);
    }
    if let Some(duration) = view_duration(declaration, "throttle_ms")? {
        view = view.throttle(duration);
    }
    if let Some(duration) = view_duration(declaration, "debounce_ms")? {
        view = view.debounce(duration);
    }
    Ok(view)
}

fn parse_stale_policy(value: &str) -> Result<ViewStalePolicy, NativeLoweringError> {
    match value {
        "hide_until_ready" => Ok(ViewStalePolicy::HideUntilReady),
        "retarget_cached" => Ok(ViewStalePolicy::RetargetCached),
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: "stale_policy".to_string(),
            expected: "hide_until_ready or retarget_cached".to_string(),
        }),
    }
}

fn view_duration(
    declaration: &ResolvedDeclaration,
    property: &str,
) -> Result<Option<Duration>, NativeLoweringError> {
    match declaration.properties.get(property) {
        None => Ok(None),
        Some(ResolvedValue::Integer(value)) if *value >= 0 => {
            Ok(Some(Duration::from_millis(*value as u64)))
        }
        Some(_) => Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "non-negative integer milliseconds".to_string(),
        }),
    }
}

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

/// The DSL's `facet` surface uses a row facet as its outer runtime container;
/// an optional column dimension is lowered as a nested column facet by the
/// registry-owned child adapter.
pub fn facet_definition() -> CoordinateLanguageDefinition<FacetRow> {
    CoordinateLanguageDefinition::new("facet", facet_schema(), |_| Ok(FacetRow))
}

pub fn facet_column_definition() -> CoordinateLanguageDefinition<FacetColumn> {
    CoordinateLanguageDefinition::new("facet_column", facet_column_schema(), |_| Ok(FacetColumn))
}

pub fn facet_wrap_definition() -> CoordinateLanguageDefinition<FacetWrap> {
    CoordinateLanguageDefinition::new("facet_wrap", facet_wrap_schema(), |_| Ok(FacetWrap))
}

pub fn repeat_rows_definition() -> CoordinateLanguageDefinition<RepeatRows> {
    CoordinateLanguageDefinition::new(
        "repeat_rows",
        repeat_schema("repeat_rows", "row", false),
        |declaration| {
            let mut coordinate = RepeatRows::new().rows(repeat_variables(declaration, "row")?);
            coordinate =
                coordinate.with_repeat_domain_coordination(repeat_coordination(declaration)?);
            Ok(coordinate)
        },
    )
}

pub fn repeat_columns_definition() -> CoordinateLanguageDefinition<RepeatColumns> {
    CoordinateLanguageDefinition::new(
        "repeat_columns",
        repeat_schema("repeat_columns", "column", false),
        |declaration| {
            let mut coordinate =
                RepeatColumns::new().columns(repeat_variables(declaration, "column")?);
            coordinate =
                coordinate.with_repeat_domain_coordination(repeat_coordination(declaration)?);
            Ok(coordinate)
        },
    )
}

pub fn repeat_grid_definition() -> CoordinateLanguageDefinition<RepeatGrid> {
    CoordinateLanguageDefinition::new(
        "repeat_grid",
        repeat_schema("repeat_grid", "row or column", true),
        |declaration| {
            let rows = repeat_variables(declaration, "row")?;
            let columns = repeat_variables(declaration, "column")?;
            if rows.is_empty() || columns.is_empty() {
                return Err(NativeLoweringError::Lowering {
                    kind: "repeat_grid".to_string(),
                    message: "repeat_grid requires at least one row and one column variable"
                        .to_string(),
                });
            }
            let mut coordinate = RepeatGrid::new().rows(rows).columns(columns);
            let coordination = repeat_coordination(declaration)?;
            coordinate = coordinate.with_repeat_domain_coordination(coordination);
            if let Some(value) = declaration.properties.get("axis_guide_visibility") {
                coordinate = coordinate
                    .axis_guide_visibility(visibility_policy("axis_guide_visibility", value)?);
            } else if matches!(
                declaration.properties.get("domain_coordination"),
                Some(ResolvedValue::String(value)) if value == "matrix"
            ) {
                coordinate = coordinate.matrix_axes();
            }
            Ok(coordinate)
        },
    )
}

pub fn repeat_wrap_definition() -> CoordinateLanguageDefinition<RepeatWrap> {
    CoordinateLanguageDefinition::new("repeat_wrap", repeat_wrap_schema(), |declaration| {
        let mut coordinate = RepeatWrap::new().items(repeat_variables(declaration, "item")?);
        coordinate = coordinate.with_repeat_domain_coordination(repeat_coordination(declaration)?);
        if declaration.properties.contains_key("columns")
            && declaration.properties.contains_key("responsive_columns")
        {
            return Err(NativeLoweringError::Lowering {
                kind: "repeat_wrap".to_string(),
                message: "columns and responsive_columns are mutually exclusive".to_string(),
            });
        }
        if let Some(value) = declaration.properties.get("columns") {
            coordinate = coordinate.columns(expr("columns", value)?);
        }
        if let Some(value) = declaration.properties.get("responsive_columns") {
            coordinate = coordinate.responsive_columns(expr("responsive_columns", value)?);
        }
        Ok(coordinate)
    })
}

fn repeat_schema(kind: &str, variable_role: &str, grid: bool) -> KindSchema {
    let mut schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, kind),
        format!("A {kind} container instantiated from ordered repeat variables."),
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "domain_coordination",
        PropertySchema::optional(
            atom(&[
                (
                    "independent",
                    "Infer domains independently for every repeated cell.",
                ),
                (
                    "matrix",
                    "Coordinate domains by repeat variable across cells.",
                ),
            ]),
            "Domain coordination policy for repeated cells.",
        ),
    )
    .child_rule(ChildRule {
        role: "variable".to_string(),
        min: if grid { 2 } else { 1 },
        max: None,
        docs: format!(
            "An ordered `{variable_role}` repeat variable with expr, title, and optional type hint."
        ),
    })
    .child_rule(ChildRule {
        role: "cell".to_string(),
        min: 1,
        max: None,
        docs: "A default or predicate-guarded repeated child plot template.".to_string(),
    });
    if grid {
        schema = schema.property(
            "axis_guide_visibility",
            axis_visibility("Axis label and title compaction across repeat-grid cells."),
        );
    }
    schema
}

fn repeat_wrap_schema() -> KindSchema {
    repeat_schema("repeat_wrap", "item", false)
        .property(
            "columns",
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Expression yielding the fixed number of physical columns.",
            ),
        )
        .property(
            "responsive_columns",
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Expression yielding the target minimum repeated-cell width in pixels.",
            ),
        )
}

fn repeat_variables(
    declaration: &ResolvedDeclaration,
    role: &str,
) -> Result<Vec<RepeatVariable>, NativeLoweringError> {
    declaration
        .children
        .iter()
        .filter(|child| child.kind == "variable" && child.variant.as_deref() == Some(role))
        .map(|child| {
            let id = child
                .source_name
                .as_ref()
                .ok_or_else(|| NativeLoweringError::Lowering {
                    kind: declaration.kind.clone(),
                    message: format!("repeat {role} variable is missing its stable id"),
                })?;
            let value =
                child
                    .properties
                    .get("expr")
                    .ok_or_else(|| NativeLoweringError::Lowering {
                        kind: declaration.kind.clone(),
                        message: format!("repeat {role} variable `{id}` requires `expr`"),
                    })?;
            let mut variable = match value {
                ResolvedValue::String(field) => {
                    RepeatVariable::new(id, datafusion::prelude::col(field))
                }
                _ => RepeatVariable::new(id, expr("expr", value)?),
            };
            if let Some(ResolvedValue::String(title)) = child.properties.get("title") {
                variable = variable.title(title);
            }
            if let Some(ResolvedValue::String(type_hint)) = child.properties.get("type") {
                variable = variable.type_hint(match type_hint.as_str() {
                    "quantitative" => RepeatTypeHint::Quantitative,
                    "temporal" => RepeatTypeHint::Temporal,
                    "ordinal" => RepeatTypeHint::Ordinal,
                    "nominal" => RepeatTypeHint::Nominal,
                    _ => {
                        return Err(NativeLoweringError::InvalidPropertyType {
                            property: "type".to_string(),
                            expected: "quantitative, temporal, ordinal, or nominal".to_string(),
                        });
                    }
                });
            }
            Ok(variable)
        })
        .collect()
}

fn repeat_coordination(
    declaration: &ResolvedDeclaration,
) -> Result<RepeatDomainCoordination, NativeLoweringError> {
    match declaration.properties.get("domain_coordination") {
        None => Ok(RepeatDomainCoordination::Independent),
        Some(ResolvedValue::String(value)) if value == "independent" => {
            Ok(RepeatDomainCoordination::Independent)
        }
        Some(ResolvedValue::String(value)) if value == "matrix" => Ok(
            RepeatDomainCoordination::by_variable(CoordinationScope::Shared),
        ),
        Some(_) => Err(NativeLoweringError::InvalidPropertyType {
            property: "domain_coordination".to_string(),
            expected: "independent or matrix".to_string(),
        }),
    }
}

fn facet_dimension_fields(wrap: bool) -> std::collections::BTreeMap<String, PropertySchema> {
    let mut fields = std::collections::BTreeMap::from([
        (
            "title".to_string(),
            PropertySchema::optional(
                ValueShape::String,
                "Facet guide title; an empty title hides it.",
            ),
        ),
        (
            "slots".to_string(),
            PropertySchema::optional(
                ValueShape::CoordinationScope,
                "Facet slot sharing scope: free, shared, or level(n).",
            ),
        ),
        (
            "empty_cells".to_string(),
            PropertySchema::optional(
                atom(&[
                    ("hole", "Do not render a subplot for an empty cell."),
                    ("empty_subplot", "Render the empty subplot structure."),
                    ("auto", "Use the runtime default empty-cell policy."),
                ]),
                "Rendering policy for empty facet cells.",
            ),
        ),
        (
            "order_by".to_string(),
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Expression used to order facet slots.",
            ),
        ),
        (
            "order".to_string(),
            PropertySchema::optional(
                atom(&[("asc", "Ascending order."), ("desc", "Descending order.")]),
                "Facet slot ordering direction.",
            ),
        ),
        (
            "position".to_string(),
            PropertySchema::optional(ValueShape::Identifier, "Facet guide position."),
        ),
        (
            "visible".to_string(),
            PropertySchema::optional(ValueShape::Boolean, "Whether the facet guide is visible."),
        ),
        (
            "axis_guide_visibility".to_string(),
            axis_visibility("Axis label and title visibility within this facet dimension."),
        ),
    ]);
    if wrap {
        fields.insert(
            "columns".to_string(),
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Expression yielding the fixed number of physical columns.",
            ),
        );
        fields.insert(
            "responsive_columns".to_string(),
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Expression yielding the target minimum leaf width in pixels.",
            ),
        );
    }
    fields
}

fn facet_schema() -> KindSchema {
    KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, "facet"),
        "A row facet, optionally containing a nested column facet for a two-dimensional grid.",
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "row",
        PropertySchema::required(
            ValueShape::ConfiguredExpression(facet_dimension_fields(false)),
            "Required outer row facet expression and configuration.",
        ),
    )
    .property(
        "column",
        PropertySchema::optional(
            ValueShape::ConfiguredExpression(facet_dimension_fields(false)),
            "Optional nested column facet expression and configuration.",
        ),
    )
    .child_rule(ChildRule {
        role: "cell".to_string(),
        min: 1,
        max: Some(1),
        docs: "The child plot instantiated for every facet cell.".to_string(),
    })
}

fn facet_column_schema() -> KindSchema {
    KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, "facet_column"),
        "A one-dimensional column facet container.",
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "column",
        PropertySchema::required(
            ValueShape::ConfiguredExpression(facet_dimension_fields(false)),
            "Column facet expression and configuration.",
        ),
    )
    .child_rule(ChildRule {
        role: "cell".to_string(),
        min: 1,
        max: Some(1),
        docs: "The child plot instantiated for every facet cell.".to_string(),
    })
}

fn facet_wrap_schema() -> KindSchema {
    KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, "facet_wrap"),
        "A wrapped one-dimensional facet whose physical columns may be fixed or responsive.",
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "facet",
        PropertySchema::required(
            ValueShape::ConfiguredExpression(facet_dimension_fields(true)),
            "Wrapped facet expression and configuration.",
        ),
    )
    .child_rule(ChildRule {
        role: "cell".to_string(),
        min: 1,
        max: Some(1),
        docs: "The child plot instantiated for every wrapped facet cell.".to_string(),
    })
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
