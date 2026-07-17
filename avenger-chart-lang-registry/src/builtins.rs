//! Bootstrap built-ins used to prove the public registry mechanism end to end.
//!
//! This is intentionally not the complete Avenger v1 inventory. The language
//! compiler plan owns the family-by-family expansion from this slice.

use std::sync::Arc;

use avenger_chart::{
    layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint},
    prelude::{Auto, Cartesian, CartesianAxis, Legend, Linear, Ordinal, PanScrollZoom, Scale},
};
use avenger_chart_core::Axis;
use avenger_chart_schema::{
    ExportSchema, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema, ValueShape,
};
use datafusion::{
    common::ScalarValue,
    logical_expr::{Expr, lit},
};

use crate::{CoordinatePack, NativeRegistry, NativeRegistryBuilder, RegistryError, ResolvedValue};

pub const BOOTSTRAP_PROFILE_LABEL: &str = "bootstrap-vertical-slice";

pub fn bootstrap_registry() -> Result<NativeRegistry, RegistryError> {
    let mut builder = NativeRegistryBuilder::new(1, BOOTSTRAP_PROFILE_LABEL);
    register_bootstrap_builtins(&mut builder)?;
    builder.build()
}

pub fn register_bootstrap_builtins(
    builder: &mut NativeRegistryBuilder,
) -> Result<(), RegistryError> {
    builder.register_coordinate_pack(cartesian_pack())?;
    builder.register_coordinate_pack(CoordinatePack::from_language_definition(
        avenger_chart_polar::language::definition(),
    ))?;
    builder.register_coordinate_pack(CoordinatePack::from_language_definition(
        avenger_chart_parallel::language::definition(),
    ))?;
    builder.register_coordinate_pack(CoordinatePack::from_language_definition(
        avenger_chart_geo::language::definition(),
    ))?;
    builder.register_coordinate_pack(CoordinatePack::from_language_definition(
        avenger_chart_treemap::language::definition(),
    ))?;
    builder.register_coordinate_pack(CoordinatePack::from_language_definition(
        avenger_chart_marks::language::zero_d_definition(),
    ))?;
    register_bootstrap_noncoordinate_builtins(builder)
}

/// Register the coordinate-independent bootstrap families. Downstream hosts
/// can use this when assembling a custom coordinate-pack set manually.
pub fn register_bootstrap_noncoordinate_builtins(
    builder: &mut NativeRegistryBuilder,
) -> Result<(), RegistryError> {
    register_transforms(builder)?;
    register_widgets(builder)?;
    register_objects(builder)
}

pub fn cartesian_pack() -> CoordinatePack<Cartesian> {
    CoordinatePack::from_language_definition(avenger_chart_cartesian::language::definition()).tool(
        "pan_scroll_zoom",
        pan_scroll_zoom_schema(),
        lower_pan_scroll_zoom,
    )
}

fn pan_scroll_zoom_schema() -> KindSchema {
    let mut tool = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Tool, "pan_scroll_zoom"),
        "Pointer-drag panning and wheel zoom for Cartesian domains.",
    )
    .export(ExportSchema {
        alias: "x_domain".to_string(),
        value_kind: "param<fixed_size_list(float64,2)>".to_string(),
        lazy: false,
        binding_property: None,
        default_property: None,
        docs: "The tool-owned current x domain.".to_string(),
    })
    .export(ExportSchema {
        alias: "y_domain".to_string(),
        value_kind: "param<fixed_size_list(float64,2)>".to_string(),
        lazy: false,
        binding_property: None,
        default_property: None,
        docs: "The tool-owned current y domain.".to_string(),
    });
    tool.compatible_coordinates.insert("cartesian".to_string());
    tool
}

fn lower_pan_scroll_zoom(
    declaration: &crate::ResolvedDeclaration,
) -> Result<Arc<dyn avenger_chart::prelude::ChartTool<Cartesian>>, RegistryError> {
    let mut tool = PanScrollZoom::cartesian();
    if let Some(source_name) = &declaration.source_name {
        tool = tool.id(source_name.clone());
    }
    Ok(Arc::new(tool))
}

fn register_transforms(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    for definition in avenger_chart_transforms::language::definitions() {
        builder.register_transform_definition(definition)?;
    }
    builder.register_transform_pipeline_definition(
        avenger_chart_transforms::language::pipeline_definition(),
    )?;
    Ok(())
}

fn register_widgets(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    for definition in avenger_chart_widgets::language::definitions() {
        builder.register_widget_definition(definition)?;
    }
    Ok(())
}

fn register_objects(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    let common_scale_properties = |schema: KindSchema| {
        schema
            .property(
                "domain",
                PropertySchema::optional(
                    ValueShape::Array(Box::new(ValueShape::Any)),
                    "Explicit scale domain values.",
                ),
            )
            .property(
                "range",
                PropertySchema::optional(
                    ValueShape::Array(Box::new(ValueShape::Any)),
                    "Explicit scale range values.",
                ),
            )
    };

    let linear = common_scale_properties(
        KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Scale, "linear"),
            "A continuous linear scale.",
        )
        .property(
            "nice",
            PropertySchema::optional(ValueShape::Boolean, "Round the domain to pleasant values."),
        )
        .property(
            "zero",
            PropertySchema::optional(ValueShape::Boolean, "Include zero in the inferred domain."),
        ),
    );
    builder.register_object(
        linear,
        Arc::new(|declaration| {
            let mut scale = Scale::<Linear>::new().into_type::<Auto>();
            scale = lower_scale(scale, declaration, true)?;
            if let Some(ResolvedValue::Boolean(value)) = declaration.properties.get("nice") {
                scale = scale._option("nice", lit(*value));
            }
            if let Some(ResolvedValue::Boolean(value)) = declaration.properties.get("zero") {
                scale = scale._option("zero", lit(*value));
            }
            Ok(Box::new(scale))
        }),
    )?;

    let ordinal = common_scale_properties(KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Scale, "ordinal"),
        "A discrete ordinal scale.",
    ));
    builder.register_object(
        ordinal,
        Arc::new(|declaration| {
            Ok(Box::new(lower_scale(
                Scale::<Ordinal>::new().into_type::<Auto>(),
                declaration,
                false,
            )?))
        }),
    )?;

    let axis = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Axis, "cartesian"),
        "A Cartesian axis configuration.",
    )
    .property(
        "title",
        PropertySchema::optional(ValueShape::SqlExpression, "Axis title."),
    )
    .property(
        "grid",
        PropertySchema::optional(ValueShape::SqlExpression, "Whether to draw grid lines."),
    )
    .property(
        "tick_count",
        PropertySchema::optional(ValueShape::SqlExpression, "Requested number of ticks."),
    )
    .property(
        "visible",
        PropertySchema::optional(ValueShape::SqlExpression, "Whether the axis is visible."),
    )
    .property(
        "position",
        PropertySchema::optional(ValueShape::SqlExpression, "Axis side position."),
    );
    builder.register_object(
        axis,
        Arc::new(|declaration| {
            let mut axis = CartesianAxis::new();
            for (name, value) in &declaration.properties {
                let expression = native_expr(value, name)?;
                axis = match name.as_str() {
                    "title" => axis.title(expression),
                    "grid" => axis.grid(expression),
                    "tick_count" => axis.tick_count(expression),
                    "visible" => axis.visible(expression),
                    "position" => axis.position(expression),
                    _ => unreachable!("registry validation checks axis properties"),
                };
            }
            let erased: Box<dyn Axis> = Box::new(axis);
            Ok(Box::new(erased))
        }),
    )?;

    let legend = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Legend, "standard"),
        "A standard chart legend configuration.",
    )
    .property(
        "title",
        PropertySchema::optional(ValueShape::SqlExpression, "Legend title."),
    )
    .property(
        "visible",
        PropertySchema::optional(ValueShape::SqlExpression, "Whether the legend is visible."),
    )
    .property(
        "position",
        PropertySchema::optional(ValueShape::SqlExpression, "Legend chrome position."),
    )
    .property(
        "orientation",
        PropertySchema::optional(ValueShape::SqlExpression, "Legend orientation."),
    )
    .property(
        "columns",
        PropertySchema::optional(ValueShape::SqlExpression, "Number of legend columns."),
    );
    builder.register_object(
        legend,
        Arc::new(|declaration| {
            let mut legend = Legend::new();
            for (name, value) in &declaration.properties {
                let expression = native_expr(value, name)?;
                legend = match name.as_str() {
                    "title" => legend.title(expression),
                    "visible" => legend.visible(expression),
                    "position" => legend.position(expression),
                    "orientation" => legend.orientation(expression),
                    "columns" => legend.columns(expression),
                    _ => unreachable!("registry validation checks legend properties"),
                };
            }
            Ok(Box::new(legend))
        }),
    )?;

    let layout = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Layout, "chart"),
        "The default chart frame layout.",
    )
    .property(
        "canvas",
        PropertySchema::optional(
            ValueShape::Any,
            "Canvas width/height constraints or `auto`.",
        ),
    )
    .property(
        "plot",
        PropertySchema::optional(
            ValueShape::Any,
            "Plot-area width/height constraints or `auto`.",
        ),
    )
    .property(
        "margins",
        PropertySchema::optional(ValueShape::Any, "Fixed chart margins."),
    );
    builder.register_object(layout, Arc::new(lower_layout))?;
    Ok(())
}

fn lower_scale(
    mut scale: Scale<Auto>,
    declaration: &crate::ResolvedDeclaration,
    continuous: bool,
) -> Result<Scale<Auto>, RegistryError> {
    if let Some(ResolvedValue::Array(domain)) = declaration.properties.get("domain") {
        let expressions = domain
            .iter()
            .enumerate()
            .map(|(index, value)| native_expr(value, &format!("domain[{index}]")))
            .collect::<Result<Vec<_>, _>>()?;
        if continuous {
            let [min, max] = expressions.as_slice() else {
                return Err(RegistryError::InvalidPropertyType {
                    property: "domain".to_string(),
                    expected: "exactly two values for a continuous scale".to_string(),
                });
            };
            scale = scale.domain_interval(min.clone(), max.clone());
        } else {
            scale = scale.domain_discrete(expressions);
        }
    }
    if let Some(ResolvedValue::Array(range)) = declaration.properties.get("range") {
        scale = scale.range_discrete(
            range
                .iter()
                .enumerate()
                .map(|(index, value)| resolved_scalar(value, &format!("range[{index}]")))
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok(scale)
}

fn lower_layout(
    declaration: &crate::ResolvedDeclaration,
) -> Result<Box<dyn std::any::Any + Send + Sync>, RegistryError> {
    let mut layout = LayoutSpec::default();
    if let Some(value) = declaration.properties.get("canvas") {
        layout = match dimensions(value, "canvas")? {
            Dimensions::Auto => layout.canvas_constraint(CanvasConstraint::None),
            Dimensions::Width(width) => layout.canvas_constraint(CanvasConstraint::Width(width)),
            Dimensions::Height(height) => {
                layout.canvas_constraint(CanvasConstraint::Height(height))
            }
            Dimensions::Fixed(width, height) => layout.canvas_size(width, height),
        };
    }
    if let Some(value) = declaration.properties.get("plot") {
        layout = match dimensions(value, "plot")? {
            Dimensions::Auto => layout.plot_constraint(PlotConstraint::Auto),
            Dimensions::Width(width) => layout.plot_constraint(PlotConstraint::Width(width)),
            Dimensions::Height(height) => layout.plot_constraint(PlotConstraint::Height(height)),
            Dimensions::Fixed(width, height) => layout.plot_size(width, height),
        };
    }
    if let Some(ResolvedValue::Object(values)) = declaration.properties.get("margins") {
        let mut margins = Margins::default();
        for (name, value) in values {
            let expression = native_expr(value, &format!("margins.{name}"))?;
            margins = match name.as_str() {
                "top" => margins.top(expression),
                "right" => margins.right(expression),
                "bottom" => margins.bottom(expression),
                "left" => margins.left(expression),
                _ => {
                    return Err(RegistryError::UnknownProperty {
                        kind: "margins".to_string(),
                        property: name.clone(),
                    });
                }
            };
        }
        layout = layout.with_margins(margins);
    }
    Ok(Box::new(layout))
}

enum Dimensions {
    Auto,
    Width(Expr),
    Height(Expr),
    Fixed(Expr, Expr),
}

fn dimensions(value: &ResolvedValue, name: &str) -> Result<Dimensions, RegistryError> {
    if matches!(value, ResolvedValue::String(value) if value == "auto") {
        return Ok(Dimensions::Auto);
    }
    let ResolvedValue::Object(values) = value else {
        return Err(RegistryError::InvalidPropertyType {
            property: name.to_string(),
            expected: "`auto` or an object with width and/or height".to_string(),
        });
    };
    let width = values
        .get("width")
        .filter(|value| !matches!(value, ResolvedValue::String(value) if value == "auto"))
        .map(|value| native_expr(value, &format!("{name}.width")))
        .transpose()?;
    let height = values
        .get("height")
        .filter(|value| !matches!(value, ResolvedValue::String(value) if value == "auto"))
        .map(|value| native_expr(value, &format!("{name}.height")))
        .transpose()?;
    match (width, height) {
        (Some(width), Some(height)) => Ok(Dimensions::Fixed(width, height)),
        (Some(width), None) => Ok(Dimensions::Width(width)),
        (None, Some(height)) => Ok(Dimensions::Height(height)),
        (None, None) => Ok(Dimensions::Auto),
    }
}

fn native_expr(value: &ResolvedValue, name: &str) -> Result<Expr, RegistryError> {
    match value {
        ResolvedValue::Boolean(value) => Ok(lit(*value)),
        ResolvedValue::Integer(value) => Ok(lit(*value)),
        ResolvedValue::Number(value) => Ok(lit(*value)),
        ResolvedValue::String(value) => Ok(lit(value.clone())),
        ResolvedValue::Scalar(value) => Ok(lit(value.clone())),
        ResolvedValue::Expr(value) => Ok(value.clone()),
        _ => Err(RegistryError::InvalidPropertyType {
            property: name.to_string(),
            expected: "scalar SQL expression".to_string(),
        }),
    }
}

fn resolved_scalar(value: &ResolvedValue, name: &str) -> Result<ScalarValue, RegistryError> {
    match value {
        ResolvedValue::Boolean(value) => Ok(ScalarValue::Boolean(Some(*value))),
        ResolvedValue::Integer(value) => Ok(ScalarValue::Int64(Some(*value))),
        ResolvedValue::Number(value) => Ok(ScalarValue::Float64(Some(*value))),
        ResolvedValue::String(value) => Ok(ScalarValue::Utf8(Some(value.clone()))),
        ResolvedValue::Scalar(value) => Ok(value.clone()),
        _ => Err(RegistryError::InvalidPropertyType {
            property: name.to_string(),
            expected: "scalar value".to_string(),
        }),
    }
}
