//! Shared Avenger-language schemas and lowerers for primitive marks.
//!
//! Coordinate crates own compatibility and position-channel inventories. This
//! module owns the primitive builder lowering so every compatible coordinate
//! uses the same implementation.

use std::collections::BTreeMap;

use avenger_chart_core::{
    ChannelDescriptor, ChannelValue, CoordinateSystem, DefaultLogicalExprNodeExt, Dodge,
    FacetDataScope, GeometrySpace, IntoPlotMark, Jitter, Mark, MarkAdjustmentTransform, Nudge,
    PatternChannelValue, PlotMark, ZeroDCoord,
};
use avenger_chart_lang_types::{
    AdjustmentLanguageDefinition, CoordinateLanguageDefinition, LoweredAdjustment,
    NativeLoweringError, NativeOutputValue, ResolvedDeclaration, ResolvedValue,
};
use avenger_chart_schema::{
    BodyMode, ChannelSchema, ChildRule, EnumValueSchema, KindSchema, NativeKindKey,
    NativeKindNamespace, PropertySchema, TransformOutputSchema, ValueShape,
};
use avenger_text::types::TextSyntaxMode;

use crate::{
    Area, Image, Line, PathMark, RasterPositionSpec, Rect, Rule, Symbol, Text, Trail,
    UniformRaster2D,
};

/// Registered transform adjustments shared by every primitive-mark family.
///
/// `adjust expr` remains a language-owned item-frame assignment block. These
/// entries own every kind-bearing `adjust` declaration and make their schemas,
/// outputs, and native lowering available through the ordinary registry.
pub fn adjustment_definitions() -> Vec<AdjustmentLanguageDefinition> {
    vec![
        nudge_adjustment_definition(),
        jitter_adjustment_definition(),
        dodge_adjustment_definition(),
    ]
}

fn adjustment_schema(kind: &str, docs: &str) -> KindSchema {
    KindSchema::new(NativeKindKey::new(NativeKindNamespace::Adjust, kind), docs)
        .allowed_parent("mark")
        .property(
            "apply",
            PropertySchema::required(
                ValueShape::Map(Box::new(ValueShape::SqlExpression)),
                "Target mark channels mapped to outputs of this bound adjustment.",
            ),
        )
        .output(TransformOutputSchema {
            name: "x".to_string(),
            shape: ValueShape::SqlExpression,
            condition_property: None,
            docs: "Adjusted item-frame x position.".to_string(),
        })
        .output(TransformOutputSchema {
            name: "y".to_string(),
            shape: ValueShape::SqlExpression,
            condition_property: None,
            docs: "Adjusted item-frame y position.".to_string(),
        })
}

fn nudge_adjustment_definition() -> AdjustmentLanguageDefinition {
    AdjustmentLanguageDefinition {
        schema: adjustment_schema(
            "nudge",
            "Offset item-frame positions by fixed horizontal and vertical pixel distances.",
        )
        .property(
            "dx",
            PropertySchema::optional(
                ValueShape::Number,
                "Horizontal pixel offset; defaults to 0.",
            )
            .with_default(0.0),
        )
        .property(
            "dy",
            PropertySchema::optional(ValueShape::Number, "Vertical pixel offset; defaults to 0.")
                .with_default(0.0),
        ),
        lowerer: |declaration, context| {
            let adjustment = Nudge::new(
                optional_f32(declaration, "dx", 0.0)?,
                optional_f32(declaration, "dy", 0.0)?,
            );
            let (transform, output) = adjustment.compile(context)?;
            Ok(LoweredAdjustment {
                transform,
                outputs: BTreeMap::from([
                    ("x".to_string(), output.x()),
                    ("y".to_string(), output.y()),
                ]),
            })
        },
    }
}

fn jitter_adjustment_definition() -> AdjustmentLanguageDefinition {
    AdjustmentLanguageDefinition {
        schema: adjustment_schema(
            "jitter",
            "Apply deterministic random displacement along one item-frame axis.",
        )
        .property(
            "axis",
            PropertySchema::optional(axis_shape(), "Displacement axis; defaults to x.")
                .with_default("x"),
        )
        .property(
            "width_px",
            PropertySchema::optional(
                ValueShape::Number,
                "Full displacement width in pixels; defaults to 1.",
            )
            .with_default(1.0),
        )
        .property(
            "seed",
            PropertySchema::optional(
                ValueShape::Integer,
                "Optional non-negative deterministic random seed.",
            ),
        ),
        lowerer: |declaration, context| {
            let mut adjustment = match optional_string(declaration, "axis", "x")? {
                "x" => Jitter::x(),
                "y" => Jitter::y(),
                axis => {
                    return Err(adjustment_error(
                        declaration,
                        format!("axis must be `x` or `y`, found `{axis}`"),
                    ));
                }
            };
            adjustment = adjustment.width_px(optional_f32(declaration, "width_px", 1.0)?);
            if let Some(seed) = optional_u64(declaration, "seed")? {
                adjustment = adjustment.seed(seed);
            }
            let (transform, output) = adjustment.compile(context)?;
            Ok(LoweredAdjustment {
                transform,
                outputs: BTreeMap::from([
                    ("x".to_string(), output.x()),
                    ("y".to_string(), output.y()),
                ]),
            })
        },
    }
}

fn dodge_adjustment_definition() -> AdjustmentLanguageDefinition {
    AdjustmentLanguageDefinition {
        schema: adjustment_schema(
            "dodge",
            "Separate items into pixel-spaced lanes according to a data field.",
        )
        .property(
            "axis",
            PropertySchema::optional(axis_shape(), "Displacement axis; defaults to x.")
                .with_default("x"),
        )
        .property(
            "by",
            PropertySchema::required(
                ValueShape::Identifier,
                "Data field whose values define dodge lanes.",
            ),
        )
        .property(
            "step_px",
            PropertySchema::optional(
                ValueShape::Number,
                "Pixel distance between adjacent lanes; defaults to 1.",
            )
            .with_default(1.0),
        ),
        lowerer: |declaration, context| {
            let mut adjustment = match optional_string(declaration, "axis", "x")? {
                "x" => Dodge::x(),
                "y" => Dodge::y(),
                axis => {
                    return Err(adjustment_error(
                        declaration,
                        format!("axis must be `x` or `y`, found `{axis}`"),
                    ));
                }
            };
            adjustment = adjustment
                .by(required_string(declaration, "by")?)
                .step_px(optional_f32(declaration, "step_px", 1.0)?);
            let (transform, output) = adjustment.compile(context)?;
            Ok(LoweredAdjustment {
                transform,
                outputs: BTreeMap::from([
                    ("x".to_string(), output.x()),
                    ("y".to_string(), output.y()),
                ]),
            })
        },
    }
}

fn axis_shape() -> ValueShape {
    ValueShape::Atom {
        values: ["x", "y"]
            .into_iter()
            .map(|value| EnumValueSchema {
                value: value.to_string(),
                docs: format!("Adjust along the {value} axis."),
            })
            .collect(),
    }
}

fn optional_f32(
    declaration: &ResolvedDeclaration,
    name: &str,
    default: f32,
) -> Result<f32, NativeLoweringError> {
    match declaration.properties.get(name) {
        None => Ok(default),
        Some(ResolvedValue::Integer(value)) => Ok(*value as f32),
        Some(ResolvedValue::Number(value)) => Ok(*value as f32),
        Some(_) => Err(adjustment_error(
            declaration,
            format!("property `{name}` must be numeric"),
        )),
    }
}

fn optional_u64(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<u64>, NativeLoweringError> {
    match declaration.properties.get(name) {
        None => Ok(None),
        Some(ResolvedValue::Integer(value)) if *value >= 0 => Ok(Some(*value as u64)),
        Some(_) => Err(adjustment_error(
            declaration,
            format!("property `{name}` must be a non-negative integer"),
        )),
    }
}

fn optional_string<'a>(
    declaration: &'a ResolvedDeclaration,
    name: &str,
    default: &'a str,
) -> Result<&'a str, NativeLoweringError> {
    match declaration.properties.get(name) {
        None => Ok(default),
        Some(ResolvedValue::String(value)) => Ok(value),
        Some(_) => Err(adjustment_error(
            declaration,
            format!("property `{name}` must be an identifier"),
        )),
    }
}

fn required_string<'a>(
    declaration: &'a ResolvedDeclaration,
    name: &str,
) -> Result<&'a str, NativeLoweringError> {
    optional_string(declaration, name, "").and_then(|value| {
        if value.is_empty() {
            Err(adjustment_error(
                declaration,
                format!("property `{name}` is required"),
            ))
        } else {
            Ok(value)
        }
    })
}

fn adjustment_error(
    declaration: &ResolvedDeclaration,
    message: impl Into<String>,
) -> NativeLoweringError {
    NativeLoweringError::Lowering {
        kind: declaration.kind.clone(),
        message: message.into(),
    }
}

/// Build the common schema shell for a primitive mark from the coordinate
/// owner's execution-channel inventory.
pub fn primitive_schema(
    coordinate: &str,
    kind: &str,
    docs: impl Into<String>,
    channels: impl IntoIterator<Item = ChannelDescriptor>,
) -> KindSchema {
    let mut schema = common_mark_schema(
        KindSchema::new(NativeKindKey::mark(coordinate, kind), docs)
            .body_mode(BodyMode::Mixed)
            .child_rule(ChildRule {
                role: "view".to_string(),
                min: 0,
                max: Some(1),
                docs: "Optional inline data view owned by this mark.".to_string(),
            })
            .child_rule(ChildRule {
                role: "adjust".to_string(),
                min: 0,
                max: None,
                docs: "Ordered render-stage expression or transform adjustment.".to_string(),
            }),
    );
    if matches!(kind, "symbol" | "rect") {
        schema = schema.child_rule(ChildRule {
            role: "derive".to_string(),
            min: 0,
            max: None,
            docs: "Primitive marks derived from each rendered source item.".to_string(),
        });
    }
    for channel in channels {
        schema = schema.channel(ChannelSchema {
            name: channel.name.to_string(),
            required: channel.required,
            shape: if channel.name == "fill_pattern" {
                ValueShape::PatternChannel
            } else {
                ValueShape::SqlExpression
            },
            docs: format!("The `{}` encoding channel.", channel.name),
        });
    }
    schema
}

/// Add the coordinate-independent state surface shared by primitive and
/// native compound marks. Keeping this vocabulary with the mark owner gives
/// every coordinate pack the same authoring contract.
pub fn common_mark_schema(schema: KindSchema) -> KindSchema {
    schema
        .property(
            "visible",
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Scalar boolean expression controlling whether the mark is rendered.",
            ),
        )
        .property(
            "details",
            PropertySchema::optional(
                ValueShape::OneOrMany(Box::new(ValueShape::Identifier)),
                "Data field names retained for interaction details and path partitioning.",
            ),
        )
        .property(
            "zindex",
            PropertySchema::optional(ValueShape::Integer, "Integer rendering order."),
        )
        .property(
            "facet_data_scope",
            PropertySchema::optional(
                ValueShape::FacetDataScope,
                "Facet visibility scope: filtered, broadcast, or level(n).",
            ),
        )
        .property(
            "geometry_space",
            PropertySchema::optional(
                ValueShape::Atom {
                    values: [
                        ("coordinate", "Build geometry before coordinate projection."),
                        (
                            "display",
                            "Build geometry after projection in display space.",
                        ),
                    ]
                    .into_iter()
                    .map(|(value, docs)| EnumValueSchema {
                        value: value.to_string(),
                        docs: docs.to_string(),
                    })
                    .collect(),
                },
                "Space in which the mark constructs geometry.",
            ),
        )
}

/// Build a primitive text schema, including the non-channel syntax property.
pub fn primitive_text_schema(
    coordinate: &str,
    docs: impl Into<String>,
    channels: impl IntoIterator<Item = ChannelDescriptor>,
) -> KindSchema {
    primitive_schema(coordinate, "text", docs, channels).property(
        "syntax",
        PropertySchema::optional(
            ValueShape::Atom {
                values: [
                    ("plain", "Render the text literally."),
                    ("typst", "Render the text as Typst markup."),
                ]
                .into_iter()
                .map(|(value, docs)| EnumValueSchema {
                    value: value.to_string(),
                    docs: docs.to_string(),
                })
                .collect(),
            },
            "Text syntax mode; defaults to `plain`.",
        ),
    )
}

/// The neutral zero-dimensional coordinate pack lives here because the core
/// coordinate type deliberately cannot depend on language contracts, while
/// its only built-in mark implementation is the primitive Symbol owner.
pub fn zero_d_definition() -> CoordinateLanguageDefinition<ZeroDCoord> {
    CoordinateLanguageDefinition::new(
        "zerod",
        KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Coordinate, "zerod"),
            "A zero-dimensional coordinate system that places marks at the plot center.",
        )
        .body_mode(BodyMode::Mixed),
        |_declaration| Ok(ZeroDCoord::new()),
    )
    .mark(
        "symbol",
        primitive_schema(
            "zerod",
            "symbol",
            "A non-spatial symbol placed at the plot center.",
            [
                "size",
                "fill",
                "stroke",
                "stroke_width",
                "shape",
                "angle",
                "opacity",
            ]
            .into_iter()
            .map(channel),
        ),
        lower_symbol::<ZeroDCoord>,
    )
    .mark(
        "text",
        primitive_text_schema(
            "zerod",
            "Text placed at the plot center.",
            [
                "text",
                "align",
                "baseline",
                "angle",
                "color",
                "font",
                "font_size",
                "font_weight",
                "font_style",
                "limit",
                "opacity",
                "defined",
            ]
            .into_iter()
            .map(channel),
        ),
        lower_text::<ZeroDCoord>,
    )
}

/// Schema for the uniform 2D raster primitive. Raster data and dimensions are
/// typed properties; visual encodings remain ordinary configured channels.
pub fn uniform_raster_schema(coordinate: &str) -> KindSchema {
    primitive_schema(
        coordinate,
        "uniform_raster_2d",
        "A uniformly binned two-dimensional raster image.",
        [
            "fill",
            "opacity",
            "opacity_by_total",
            "null_color",
            "non_finite_color",
        ]
        .into_iter()
        .map(channel),
    )
    .channel(ChannelSchema {
        name: "opacity_by_total".to_string(),
        required: false,
        shape: ValueShape::ChannelConfig,
        docs: "Configuration for the internal per-pixel total-to-opacity channel.".to_string(),
    })
    .property(
        "raster",
        PropertySchema::required(
            ValueShape::SqlExpression,
            "Raster struct expression, usually a rasterize_2d output handle.",
        ),
    )
    .property(
        "x",
        PropertySchema::optional(
            ValueShape::RasterDimensionChannel,
            "Configured raster x-dimension handle.",
        ),
    )
    .property(
        "y",
        PropertySchema::optional(
            ValueShape::RasterDimensionChannel,
            "Configured raster y-dimension handle.",
        ),
    )
    .property(
        "fill_by",
        PropertySchema::optional(
            ValueShape::RasterDimensionChannel,
            "Categorical raster plane dimension that drives the fill scale.",
        ),
    )
    .property(
        "smooth",
        PropertySchema::optional(ValueShape::Boolean, "Enable smooth image sampling."),
    )
}

pub fn lower_uniform_raster<C>(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<PlotMark<C>>, NativeLoweringError>
where
    C: CoordinateSystem,
    UniformRaster2D<C>: Mark<C>,
{
    let raster = match declaration.get("raster")? {
        ResolvedValue::Expr(expr) => expr.clone(),
        ResolvedValue::Output(NativeOutputValue::Expr(expr)) => expr.clone(),
        _ => {
            return Err(NativeLoweringError::InvalidPropertyType {
                property: "raster".to_string(),
                expected: "raster expression output".to_string(),
            });
        }
    };
    let mut mark = UniformRaster2D::<C>::new().with_mark_effects(declaration.mark_effects.clone());
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }
    let fill = declaration
        .properties
        .get("fill")
        .map(|value| ordinary_channel("fill", value))
        .transpose()?;
    let x = raster_position(declaration.properties.get("x"), "x")?;
    let y = raster_position(declaration.properties.get("y"), "y")?;
    let fill_by = raster_dimension_channel(declaration.properties.get("fill_by"), "fill_by")?;
    let opacity_by_total = declaration
        .properties
        .get("opacity_by_total")
        .map(|value| ordinary_channel("opacity_by_total", value))
        .transpose()?;
    mark =
        mark.configure_raster_with_overlay(raster, fill, Some((x, y)), fill_by, opacity_by_total);
    if let Some(value) = declaration.properties.get("opacity") {
        mark = mark.with_channel_value("opacity", ordinary_channel("opacity", value)?);
    }
    if let Some(value) = declaration.properties.get("null_color") {
        mark = mark.null_color(ordinary_channel("null_color", value)?);
    }
    if let Some(value) = declaration.properties.get("non_finite_color") {
        mark = mark.non_finite_color(ordinary_channel("non_finite_color", value)?);
    }
    if let Some(ResolvedValue::Boolean(value)) = declaration.properties.get("smooth") {
        mark = mark.smooth(*value);
    }
    apply_common_mark_state::<C, _>(&mut mark, declaration)?;
    Ok(mark.into_plot_marks())
}

fn raster_position(
    value: Option<&ResolvedValue>,
    property: &str,
) -> Result<Option<RasterPositionSpec>, NativeLoweringError> {
    Ok(
        raster_dimension_channel(value, property)?.map(|(dimension, channel_value)| {
            RasterPositionSpec {
                dim: dimension,
                channel_value,
            }
        }),
    )
}

fn raster_dimension_channel(
    value: Option<&ResolvedValue>,
    property: &str,
) -> Result<Option<(avenger_chart_core::RasterDim, ChannelValue)>, NativeLoweringError> {
    match value {
        None => Ok(None),
        Some(ResolvedValue::RasterDimensionChannel { dimension, channel }) => {
            Ok(Some((dimension.clone(), channel.as_ref().clone())))
        }
        Some(_) => Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "configured raster dimension".to_string(),
        }),
    }
}

/// Construct a runtime-channel descriptor without duplicating boilerplate in
/// coordinate language packs.
pub const fn channel(name: &'static str) -> ChannelDescriptor {
    ChannelDescriptor {
        name,
        required: false,
        default_value: None,
        allow_column_ref: true,
    }
}

pub const fn required_channel(name: &'static str) -> ChannelDescriptor {
    ChannelDescriptor {
        name,
        required: true,
        default_value: None,
        allow_column_ref: true,
    }
}

fn ordinary_channel(
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
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "resolved channel value".to_string(),
        }),
    }
}

fn pattern_channel(
    property: &str,
    value: &ResolvedValue,
) -> Result<PatternChannelValue, NativeLoweringError> {
    match value {
        ResolvedValue::Pattern(value) => Ok(value.clone()),
        ResolvedValue::Expr(expr) => Ok(expr.clone().into()),
        ResolvedValue::Channel(value) => Ok(value.channel_value().clone().into()),
        ResolvedValue::Output(NativeOutputValue::Expr(expr)) => Ok(expr.clone().into()),
        ResolvedValue::Output(NativeOutputValue::Channel(value)) => Ok(value.clone().into()),
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "resolved pattern channel value".to_string(),
        }),
    }
}

/// Apply the coordinate-independent mark properties after schema-directed
/// native lowering has separated them from encoding channels.
pub fn apply_common_mark_state<C, M>(
    mark: &mut M,
    declaration: &ResolvedDeclaration,
) -> Result<(), NativeLoweringError>
where
    C: CoordinateSystem,
    M: Mark<C>,
{
    let state = mark.state_mut();
    if let Some(value) = declaration.properties.get("visible") {
        let ResolvedValue::Expr(expr) = value else {
            return Err(invalid_common_property(
                "visible",
                "scalar boolean expression",
            ));
        };
        state.visible = Some(
            DefaultLogicalExprNodeExt::from_default_expr(expr.clone()).map_err(|error| {
                NativeLoweringError::Lowering {
                    kind: declaration.kind.clone(),
                    message: format!("failed to serialize mark visibility: {error}"),
                }
            })?,
        );
    }
    if let Some(value) = declaration.properties.get("details") {
        let values = match value {
            ResolvedValue::String(value) => vec![value.clone()],
            ResolvedValue::Array(values) => values
                .iter()
                .map(|value| match value {
                    ResolvedValue::String(value) => Ok(value.clone()),
                    _ => Err(invalid_common_property("details", "field-name array")),
                })
                .collect::<Result<_, _>>()?,
            _ => return Err(invalid_common_property("details", "field name or array")),
        };
        state.details = Some(values);
    }
    if let Some(value) = declaration.properties.get("zindex") {
        let ResolvedValue::Integer(value) = value else {
            return Err(invalid_common_property("zindex", "32-bit integer"));
        };
        state.zindex = Some(
            (*value)
                .try_into()
                .map_err(|_| invalid_common_property("zindex", "32-bit integer"))?,
        );
    }
    if let Some(value) = declaration.properties.get("facet_data_scope") {
        let ResolvedValue::Integer(value) = value else {
            return Err(invalid_common_property(
                "facet_data_scope",
                "resolved facet scope level",
            ));
        };
        let level = (*value).try_into().map_err(|_| {
            invalid_common_property("facet_data_scope", "scope level from 0 through 255")
        })?;
        state.facet_data_scope = FacetDataScope::level(level);
    }
    if let Some(value) = declaration.properties.get("geometry_space") {
        let ResolvedValue::String(value) = value else {
            return Err(invalid_common_property(
                "geometry_space",
                "coordinate or display",
            ));
        };
        state.geometry_space = Some(match value.as_str() {
            "coordinate" => GeometrySpace::Coordinate,
            "display" => GeometrySpace::Display,
            _ => {
                return Err(invalid_common_property(
                    "geometry_space",
                    "coordinate or display",
                ));
            }
        });
    }
    Ok(())
}

pub fn is_common_mark_property(name: &str) -> bool {
    matches!(
        name,
        "visible" | "details" | "zindex" | "facet_data_scope" | "geometry_space"
    )
}

fn invalid_common_property(property: &str, expected: &str) -> NativeLoweringError {
    NativeLoweringError::InvalidPropertyType {
        property: property.to_string(),
        expected: expected.to_string(),
    }
}

macro_rules! primitive_lowerer {
    ($function:ident, $mark:ident) => {
        pub fn $function<C>(
            declaration: &ResolvedDeclaration,
        ) -> Result<Vec<PlotMark<C>>, NativeLoweringError>
        where
            C: CoordinateSystem,
            $mark<C>: Mark<C>,
        {
            let mut mark = $mark::<C>::new().with_mark_effects(declaration.mark_effects.clone());
            if let Some(source_name) = &declaration.source_name {
                mark = mark.id(source_name.clone());
            }
            for (name, value) in &declaration.properties {
                if is_common_mark_property(name) {
                    continue;
                }
                mark = if name == "fill_pattern" {
                    mark.with_pattern_channel_value(name, pattern_channel(name, value)?)
                } else {
                    mark.with_channel_value(name, ordinary_channel(name, value)?)
                };
            }
            apply_common_mark_state::<C, _>(&mut mark, declaration)?;
            Ok(mark.into_plot_marks())
        }
    };
}

primitive_lowerer!(lower_area, Area);
primitive_lowerer!(lower_image, Image);
primitive_lowerer!(lower_line, Line);
primitive_lowerer!(lower_path, PathMark);
primitive_lowerer!(lower_rect, Rect);
primitive_lowerer!(lower_rule, Rule);
primitive_lowerer!(lower_symbol, Symbol);
primitive_lowerer!(lower_trail, Trail);

pub fn lower_text<C>(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<PlotMark<C>>, NativeLoweringError>
where
    C: CoordinateSystem,
    Text<C>: Mark<C>,
{
    let mut mark = Text::<C>::new().with_mark_effects(declaration.mark_effects.clone());
    if let Some(source_name) = &declaration.source_name {
        mark = mark.id(source_name.clone());
    }
    for (name, value) in &declaration.properties {
        if is_common_mark_property(name) {
            continue;
        }
        if name == "syntax" {
            let ResolvedValue::String(mode) = value else {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: name.clone(),
                    expected: "`plain` or `typst`".to_string(),
                });
            };
            mark = match mode.as_str() {
                "plain" => mark.syntax_mode(TextSyntaxMode::Plain),
                "typst" => mark.syntax_mode(TextSyntaxMode::TypstMarkup),
                _ => {
                    return Err(NativeLoweringError::Lowering {
                        kind: declaration.kind.clone(),
                        message: format!("unsupported text syntax `{mode}`"),
                    });
                }
            };
        } else {
            let channel = ordinary_channel(name, value)?;
            mark = mark.with_channel_value(
                name,
                if name == "text" {
                    channel.no_scale()
                } else {
                    channel
                },
            )
        }
    }
    apply_common_mark_state::<C, _>(&mut mark, declaration)?;
    Ok(mark.into_plot_marks())
}
