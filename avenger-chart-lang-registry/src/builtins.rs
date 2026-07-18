//! Bootstrap built-ins used to prove the public registry mechanism end to end.
//!
//! This is intentionally not the complete Avenger v1 inventory. The language
//! compiler plan owns the family-by-family expansion from this slice.

use std::sync::Arc;

use avenger_chart::{
    layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint},
    prelude::Cartesian,
};
use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema, ValueShape,
};
use datafusion::logical_expr::{Expr, lit};

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
    let mut pack =
        CoordinatePack::from_language_definition(avenger_chart_cartesian::language::definition());
    for definition in avenger_chart_marks_statistical::language::definitions() {
        let avenger_chart_lang_types::MarkLanguageDefinition {
            kind,
            schema,
            lowerer,
        } = definition;
        pack = pack.mark(kind, schema, move |declaration| {
            lowerer(declaration).map_err(RegistryError::from)
        });
    }
    for definition in avenger_chart_tools::language::definitions() {
        let avenger_chart_lang_types::ToolLanguageDefinition {
            kind,
            schema,
            lowerer,
        } = definition;
        pack = pack.tool(kind, schema, move |declaration| {
            lowerer(declaration).map_err(RegistryError::from)
        });
    }
    pack
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
    for definition in avenger_chart_scales::language::definitions() {
        builder.register_object_definition(definition)?;
    }
    builder.register_object_definition(avenger_chart_cartesian::language::axis_definition())?;
    builder.register_object_definition(avenger_chart_polar::language::axis_definition())?;
    builder.register_object_definition(avenger_chart_legend::language::definition())?;

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
