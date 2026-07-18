//! Bootstrap built-ins used to prove the public registry mechanism end to end.
//!
//! This is intentionally not the complete Avenger v1 inventory. The language
//! compiler plan owns the family-by-family expansion from this slice.

use std::sync::Arc;

use avenger_chart::{
    facet::marks::{
        FacetColChannelConfig, FacetColumnSubplotChannels, FacetRowChannelConfig,
        FacetRowSubplotChannels, FacetWrapChannelConfig, FacetWrapSubplotChannels,
    },
    layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint},
    prelude::{
        Cartesian, FacetColumn, FacetRow, FacetWrap, Plot, RepeatColumns, RepeatGrid, RepeatRows,
        RepeatWrap, Subplot,
    },
};
use avenger_chart_core::{
    AxisGuideVisibilityPolicy, ChildPlotFurnishings, CoordinateSystem, CoordinationScope,
    FacetEmptyCellPolicy, SubplotChildPlotSpec, SubplotContainerCoordinateSystem, TitleSpec,
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
    register_concat_coordinates(builder)?;
    register_facet_coordinates(builder)?;
    register_repeat_coordinates(builder)?;
    register_bootstrap_noncoordinate_builtins(builder)
}

fn register_repeat_coordinates(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(avenger_chart::language::repeat_rows_definition())
            .child_plots(lower_repeat_rows_child)
            .children_use_parent_data_context(),
    )?;
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(
            avenger_chart::language::repeat_columns_definition(),
        )
        .child_plots(lower_repeat_columns_child)
        .children_use_parent_data_context(),
    )?;
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(avenger_chart::language::repeat_grid_definition())
            .child_plots(lower_repeat_grid_child)
            .children_use_parent_data_context(),
    )?;
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(avenger_chart::language::repeat_wrap_definition())
            .child_plots(lower_repeat_wrap_child)
            .children_use_parent_data_context(),
    )?;
    Ok(())
}

fn lower_repeat_rows_child(
    plot: Plot<RepeatRows>,
    child: Box<dyn SubplotChildPlotSpec>,
    placement: &crate::ResolvedDeclaration,
    parent: &crate::ResolvedPlot,
) -> Result<Plot<RepeatRows>, RegistryError> {
    validate_repeat_defaults(parent)?;
    let (when, furnishings) = repeat_cell_options(placement)?;
    Ok(plot.configure_coord(|coordinate| match when {
        Some(when) => coordinate.cell_when_erased(when, child, furnishings),
        None => coordinate.cell_erased(child, furnishings),
    }))
}

fn lower_repeat_columns_child(
    plot: Plot<RepeatColumns>,
    child: Box<dyn SubplotChildPlotSpec>,
    placement: &crate::ResolvedDeclaration,
    parent: &crate::ResolvedPlot,
) -> Result<Plot<RepeatColumns>, RegistryError> {
    validate_repeat_defaults(parent)?;
    let (when, furnishings) = repeat_cell_options(placement)?;
    Ok(plot.configure_coord(|coordinate| match when {
        Some(when) => coordinate.cell_when_erased(when, child, furnishings),
        None => coordinate.cell_erased(child, furnishings),
    }))
}

fn lower_repeat_grid_child(
    plot: Plot<RepeatGrid>,
    child: Box<dyn SubplotChildPlotSpec>,
    placement: &crate::ResolvedDeclaration,
    parent: &crate::ResolvedPlot,
) -> Result<Plot<RepeatGrid>, RegistryError> {
    validate_repeat_defaults(parent)?;
    let (when, furnishings) = repeat_cell_options(placement)?;
    Ok(plot.configure_coord(|coordinate| match when {
        Some(when) => coordinate.cell_when_erased(when, child, furnishings),
        None => coordinate.cell_erased(child, furnishings),
    }))
}

fn lower_repeat_wrap_child(
    plot: Plot<RepeatWrap>,
    child: Box<dyn SubplotChildPlotSpec>,
    placement: &crate::ResolvedDeclaration,
    parent: &crate::ResolvedPlot,
) -> Result<Plot<RepeatWrap>, RegistryError> {
    validate_repeat_defaults(parent)?;
    let (when, furnishings) = repeat_cell_options(placement)?;
    Ok(plot.configure_coord(|coordinate| match when {
        Some(when) => coordinate.cell_when_erased(when, child, furnishings),
        None => coordinate.cell_erased(child, furnishings),
    }))
}

fn validate_repeat_defaults(parent: &crate::ResolvedPlot) -> Result<(), RegistryError> {
    if parent
        .children
        .iter()
        .filter(|child| !child.placement.properties.contains_key("when"))
        .count()
        > 1
    {
        return Err(RegistryError::Lowering {
            kind: parent.coordinate.kind.clone(),
            message: "repeat containers accept at most one unguarded default cell".to_string(),
        });
    }
    Ok(())
}

fn repeat_cell_options(
    placement: &crate::ResolvedDeclaration,
) -> Result<(Option<Expr>, ChildPlotFurnishings), RegistryError> {
    let when = placement
        .properties
        .get("when")
        .map(|value| native_expr(value, "when"))
        .transpose()?;
    let mut furnishings = ChildPlotFurnishings::default();
    if let Some(label) = placement.properties.get("label") {
        furnishings.caption = Some(TitleSpec::new(native_expr(label, "label")?));
    }
    Ok((when, furnishings))
}

fn register_concat_coordinates(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(avenger_chart::language::hconcat_definition())
            .child_plots(lower_concat_child),
    )?;
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(avenger_chart::language::vconcat_definition())
            .child_plots(lower_concat_child),
    )?;
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(avenger_chart::language::grid_concat_definition())
            .child_plots(lower_concat_child),
    )?;
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(avenger_chart::language::wrap_concat_definition())
            .child_plots(lower_concat_child),
    )?;
    Ok(())
}

fn register_facet_coordinates(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(avenger_chart::language::facet_definition())
            .child_plots(lower_facet_child)
            .children_use_parent_data_context(),
    )?;
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(
            avenger_chart::language::facet_column_definition(),
        )
        .child_plots(lower_facet_column_child)
        .children_use_parent_data_context(),
    )?;
    builder.register_coordinate_pack(
        CoordinatePack::from_language_definition(avenger_chart::language::facet_wrap_definition())
            .child_plots(lower_facet_wrap_child)
            .children_use_parent_data_context(),
    )?;
    Ok(())
}

fn lower_facet_child(
    plot: Plot<FacetRow>,
    mut child: Box<dyn SubplotChildPlotSpec>,
    placement: &crate::ResolvedDeclaration,
    parent: &crate::ResolvedPlot,
) -> Result<Plot<FacetRow>, RegistryError> {
    if let Some(column) = parent.coordinate.properties.get("column") {
        let (expr, config) = configured_facet_dimension("column", column)?;
        let inner = Subplot::<FacetColumn>::new(child)
            .column_with(expr, |options| apply_facet_col(options, config));
        child = Box::new(Plot::<FacetColumn>::new().mark(inner));
    }
    let row = parent
        .coordinate
        .properties
        .get("row")
        .ok_or_else(|| missing_property("facet", "row"))?;
    let (expr, config) = configured_facet_dimension("row", row)?;
    let subplot = apply_subplot_identity(
        Subplot::<FacetRow>::new(child).row_with(expr, |options| apply_facet_row(options, config)),
        placement,
    )?;
    Ok(plot.mark(subplot))
}

fn lower_facet_column_child(
    plot: Plot<FacetColumn>,
    child: Box<dyn SubplotChildPlotSpec>,
    placement: &crate::ResolvedDeclaration,
    parent: &crate::ResolvedPlot,
) -> Result<Plot<FacetColumn>, RegistryError> {
    let column = parent
        .coordinate
        .properties
        .get("column")
        .ok_or_else(|| missing_property("facet_column", "column"))?;
    let (expr, config) = configured_facet_dimension("column", column)?;
    let subplot = apply_subplot_identity(
        Subplot::<FacetColumn>::new(child)
            .column_with(expr, |options| apply_facet_col(options, config)),
        placement,
    )?;
    Ok(plot.mark(subplot))
}

fn lower_facet_wrap_child(
    plot: Plot<FacetWrap>,
    child: Box<dyn SubplotChildPlotSpec>,
    placement: &crate::ResolvedDeclaration,
    parent: &crate::ResolvedPlot,
) -> Result<Plot<FacetWrap>, RegistryError> {
    let facet = parent
        .coordinate
        .properties
        .get("facet")
        .ok_or_else(|| missing_property("facet_wrap", "facet"))?;
    let (expr, config) = configured_facet_dimension("facet", facet)?;
    if config.contains_key("columns") && config.contains_key("responsive_columns") {
        return Err(RegistryError::Lowering {
            kind: "facet_wrap".to_string(),
            message: "facet columns and responsive_columns are mutually exclusive".to_string(),
        });
    }
    let subplot = apply_subplot_identity(
        Subplot::<FacetWrap>::new(child)
            .wrap_with(expr, |options| apply_facet_wrap(options, config)),
        placement,
    )?;
    Ok(plot.mark(subplot))
}

fn apply_subplot_identity<C: CoordinateSystem>(
    mut subplot: Subplot<C>,
    placement: &crate::ResolvedDeclaration,
) -> Result<Subplot<C>, RegistryError> {
    if let Some(name) = &placement.source_name {
        subplot = subplot.name(name.clone());
    }
    if let Some(label) = placement.properties.get("label") {
        subplot = subplot.caption(native_expr(label, "label")?);
    }
    Ok(subplot)
}

fn configured_facet_dimension<'a>(
    property: &str,
    value: &'a ResolvedValue,
) -> Result<
    (
        avenger_chart_core::ChannelValue,
        &'a indexmap::IndexMap<String, ResolvedValue>,
    ),
    RegistryError,
> {
    let ResolvedValue::Configured { head, properties } = value else {
        return Err(RegistryError::InvalidPropertyType {
            property: property.to_string(),
            expected: "a configured facet expression".to_string(),
        });
    };
    let ResolvedValue::Channel(channel) = head.as_ref() else {
        return Err(RegistryError::InvalidPropertyType {
            property: property.to_string(),
            expected: "a configured facet channel".to_string(),
        });
    };
    Ok((channel.channel_value().clone(), properties))
}

fn apply_facet_row(
    mut options: FacetRowChannelConfig,
    config: &indexmap::IndexMap<String, ResolvedValue>,
) -> FacetRowChannelConfig {
    if let Some(scope) = facet_scope(config.get("slots")) {
        options = options.with_slot_sharing(scope);
    }
    if let Some(policy) = empty_cell_policy(config.get("empty_cells")) {
        options = options.empty_cell_policy(policy);
    }
    if let Some(expr) = config.get("order_by").and_then(resolved_expr) {
        options = options.order_by(expr);
    }
    if config.get("order").and_then(resolved_string) == Some("desc") {
        options = options.order_desc();
    }
    if let Some(policy) = config
        .get("axis_guide_visibility")
        .and_then(resolved_string)
    {
        options = options.axis_guide_visibility(facet_axis_visibility(policy));
    }
    apply_row_guide(options, config)
}

fn apply_row_guide(
    options: FacetRowChannelConfig,
    config: &indexmap::IndexMap<String, ResolvedValue>,
) -> FacetRowChannelConfig {
    options.guide(|mut guide| {
        if let Some(title) = config.get("title").and_then(resolved_string) {
            guide = guide.title(title);
        }
        if let Some(position) = config.get("position").and_then(resolved_string) {
            guide = guide.position(position);
        }
        if let Some(visible) = config.get("visible").and_then(resolved_bool) {
            guide = guide.visible(visible);
        }
        guide
    })
}

fn apply_facet_col(
    mut options: FacetColChannelConfig,
    config: &indexmap::IndexMap<String, ResolvedValue>,
) -> FacetColChannelConfig {
    if let Some(scope) = facet_scope(config.get("slots")) {
        options = options.with_slot_sharing(scope);
    }
    if let Some(policy) = empty_cell_policy(config.get("empty_cells")) {
        options = options.empty_cell_policy(policy);
    }
    if let Some(expr) = config.get("order_by").and_then(resolved_expr) {
        options = options.order_by(expr);
    }
    if config.get("order").and_then(resolved_string) == Some("desc") {
        options = options.order_desc();
    }
    if let Some(policy) = config
        .get("axis_guide_visibility")
        .and_then(resolved_string)
    {
        options = options.axis_guide_visibility(facet_axis_visibility(policy));
    }
    options.guide(|mut guide| {
        if let Some(title) = config.get("title").and_then(resolved_string) {
            guide = guide.title(title);
        }
        if let Some(position) = config.get("position").and_then(resolved_string) {
            guide = guide.position(position);
        }
        if let Some(visible) = config.get("visible").and_then(resolved_bool) {
            guide = guide.visible(visible);
        }
        guide
    })
}

fn apply_facet_wrap(
    mut options: FacetWrapChannelConfig,
    config: &indexmap::IndexMap<String, ResolvedValue>,
) -> FacetWrapChannelConfig {
    if let Some(scope) = facet_scope(config.get("slots")) {
        options = options.with_slot_sharing(scope);
    }
    if let Some(policy) = empty_cell_policy(config.get("empty_cells")) {
        options = options.empty_cell_policy(policy);
    }
    if let Some(expr) = config.get("order_by").and_then(resolved_expr) {
        options = options.order_by(expr);
    }
    if config.get("order").and_then(resolved_string) == Some("desc") {
        options = options.order_desc();
    }
    if let Some(expr) = config.get("columns").and_then(resolved_expr) {
        options = options.columns(expr);
    }
    if let Some(expr) = config.get("responsive_columns").and_then(resolved_expr) {
        options = options.responsive_columns(expr);
    }
    if let Some(policy) = config
        .get("axis_guide_visibility")
        .and_then(resolved_string)
    {
        options = options.axis_guide_visibility(facet_axis_visibility(policy));
    }
    options.guide(|mut guide| {
        if let Some(title) = config.get("title").and_then(resolved_string) {
            guide = guide.title(title);
        }
        if let Some(position) = config.get("position").and_then(resolved_string) {
            guide = guide.position(position);
        }
        if let Some(visible) = config.get("visible").and_then(resolved_bool) {
            guide = guide.visible(visible);
        }
        guide
    })
}

fn facet_scope(value: Option<&ResolvedValue>) -> Option<CoordinationScope> {
    match value {
        Some(ResolvedValue::String(value)) if value == "shared" => Some(CoordinationScope::Shared),
        Some(ResolvedValue::String(value)) if value == "free" => Some(CoordinationScope::Free),
        Some(ResolvedValue::Integer(level)) => {
            u8::try_from(*level).ok().map(CoordinationScope::Level)
        }
        _ => None,
    }
}

fn empty_cell_policy(value: Option<&ResolvedValue>) -> Option<FacetEmptyCellPolicy> {
    match value.and_then(resolved_string) {
        Some("hole") => Some(FacetEmptyCellPolicy::Hole),
        Some("empty_subplot") => Some(FacetEmptyCellPolicy::EmptySubplot),
        Some("auto") => Some(FacetEmptyCellPolicy::Auto),
        _ => None,
    }
}

fn facet_axis_visibility(value: &str) -> AxisGuideVisibilityPolicy {
    match value {
        "all" => AxisGuideVisibilityPolicy::All,
        "outer_edges" => AxisGuideVisibilityPolicy::OuterEdges,
        "outer_for_equivalent_domain_groups" => {
            AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups
        }
        _ => AxisGuideVisibilityPolicy::Auto,
    }
}

fn resolved_expr(value: &ResolvedValue) -> Option<Expr> {
    match value {
        ResolvedValue::Expr(expr) => Some(expr.clone()),
        ResolvedValue::Scalar(value) => Some(lit(value.clone())),
        ResolvedValue::Integer(value) => Some(lit(*value)),
        ResolvedValue::Number(value) => Some(lit(*value)),
        ResolvedValue::String(value) => Some(lit(value.clone())),
        ResolvedValue::Boolean(value) => Some(lit(*value)),
        _ => None,
    }
}

fn resolved_string(value: &ResolvedValue) -> Option<&str> {
    match value {
        ResolvedValue::String(value) => Some(value),
        _ => None,
    }
}

fn resolved_bool(value: &ResolvedValue) -> Option<bool> {
    match value {
        ResolvedValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn missing_property(kind: &str, property: &str) -> RegistryError {
    RegistryError::Lowering {
        kind: kind.to_string(),
        message: format!("missing required `{property}` property"),
    }
}

fn lower_concat_child<C: CoordinateSystem + SubplotContainerCoordinateSystem>(
    plot: Plot<C>,
    child: Box<dyn SubplotChildPlotSpec>,
    placement: &crate::ResolvedDeclaration,
    _parent: &crate::ResolvedPlot,
) -> Result<Plot<C>, RegistryError> {
    let mut subplot = Subplot::<C>::new(child);
    if let Some(name) = &placement.source_name {
        subplot = subplot.name(name.clone());
    }
    if let Some(label) = placement.properties.get("label") {
        subplot = subplot.caption(native_expr(label, "label")?);
    }
    let row = optional_usize(placement, "row")?;
    let column = optional_usize(placement, "column")?;
    match (row, column) {
        (Some(row), Some(column)) => subplot = subplot.at(row, column),
        (None, None) => {}
        _ => {
            return Err(RegistryError::Lowering {
                kind: "cell".to_string(),
                message: "grid placement requires both row and column".to_string(),
            });
        }
    }
    if let Some(span) = optional_usize(placement, "row_span")? {
        subplot = subplot.grid_row_span(span);
    }
    if let Some(span) = optional_usize(placement, "column_span")? {
        subplot = subplot.grid_column_span(span);
    }
    Ok(plot.mark(subplot))
}

fn optional_usize(
    declaration: &crate::ResolvedDeclaration,
    property: &str,
) -> Result<Option<usize>, RegistryError> {
    let Some(value) = declaration.properties.get(property) else {
        return Ok(None);
    };
    let ResolvedValue::Integer(value) = value else {
        return Err(RegistryError::InvalidPropertyType {
            property: property.to_string(),
            expected: "a non-negative integer".to_string(),
        });
    };
    usize::try_from(*value)
        .map(Some)
        .map_err(|_| RegistryError::InvalidPropertyType {
            property: property.to_string(),
            expected: "a non-negative integer".to_string(),
        })
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
