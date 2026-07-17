//! Avenger-language registration for treemap coordinates and marks.

use avenger_chart_core::{ChannelValue, IntoPlotMark};
use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, ResolvedDeclaration, ResolvedValue,
};
use avenger_chart_marks::language::{channel, primitive_schema};
use avenger_chart_schema::{
    BodyMode, EnumValueSchema, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema,
    ValueShape,
};

use crate::{TreeHeader, TreeLabel, TreeLabelFit, TreeRect, TreeRectNodeMode, Treemap};

const RECT_CHANNELS: &[&str] = &[
    "fill",
    "stroke",
    "stroke_width",
    "opacity",
    "corner_radius",
    "u",
    "u2",
    "v",
    "v2",
];
const LABEL_CHANNELS: &[&str] = &[
    "text",
    "color",
    "font_size",
    "opacity",
    "align",
    "baseline",
    "font",
    "font_weight",
    "font_style",
];
const HEADER_CHANNELS: &[&str] = &[
    "fill",
    "stroke",
    "stroke_width",
    "opacity",
    "text",
    "text_color",
    "font_size",
    "font",
    "font_weight",
    "font_style",
];

pub fn definition() -> CoordinateLanguageDefinition<Treemap> {
    CoordinateLanguageDefinition::new("treemap", coordinate_schema(), lower_treemap)
        .mark("tree_rect", tree_rect_schema(), lower_tree_rect)
        .mark("tree_label", tree_label_schema(), lower_tree_label)
        .mark("tree_header", tree_header_schema(), lower_tree_header)
}

fn coordinate_schema() -> KindSchema {
    KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, "treemap"),
        "A hierarchical treemap layout coordinate system.",
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "path",
        PropertySchema::required(
            ValueShape::Array(Box::new(ValueShape::SqlExpression)),
            "Ordered hierarchy-level expressions from root to leaf.",
        ),
    )
    .property(
        "value",
        PropertySchema::required(
            ValueShape::SqlExpression,
            "Non-negative leaf weight expression used for area allocation.",
        ),
    )
    .property(
        "root_path_id",
        PropertySchema::optional(
            ValueShape::String,
            "Initial visible hierarchy root path id.",
        ),
    )
    .property(
        "display_levels",
        PropertySchema::optional(
            ValueShape::Integer,
            "Maximum number of hierarchy levels displayed below the current root.",
        ),
    )
}

fn lower_treemap(declaration: &ResolvedDeclaration) -> Result<Treemap, NativeLoweringError> {
    let ResolvedValue::Array(path) = declaration.get("path")? else {
        unreachable!("schema validates treemap path")
    };
    let path = path
        .iter()
        .map(|value| match value {
            ResolvedValue::Expr(value) => Ok(value.clone()),
            _ => Err(NativeLoweringError::InvalidPropertyType {
                property: "path".to_string(),
                expected: "SQL expression array".to_string(),
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let value = match declaration.get("value")? {
        ResolvedValue::Expr(value) => value.clone(),
        _ => {
            return Err(NativeLoweringError::InvalidPropertyType {
                property: "value".to_string(),
                expected: "SQL expression".to_string(),
            });
        }
    };
    let mut treemap = Treemap::new().path(path).value(value);
    if let Some(ResolvedValue::String(value)) = declaration.properties.get("root_path_id") {
        treemap = treemap.root_path_id(value.clone());
    }
    if let Some(ResolvedValue::Integer(value)) = declaration.properties.get("display_levels") {
        treemap = treemap.display_levels((*value).try_into().map_err(|_| {
            NativeLoweringError::InvalidPropertyType {
                property: "display_levels".to_string(),
                expected: "non-negative integer".to_string(),
            }
        })?);
    }
    Ok(treemap)
}

fn node_mode_property() -> PropertySchema {
    PropertySchema::optional(
        ValueShape::Union(vec![
            ValueShape::Atom {
                values: [
                    ("leaves", "Render visible terminal nodes."),
                    ("all_visible", "Render every visible hierarchy node."),
                ]
                .into_iter()
                .map(|(value, docs)| EnumValueSchema {
                    value: value.to_string(),
                    docs: docs.to_string(),
                })
                .collect(),
            },
            ValueShape::Integer,
        ]),
        "Node set to render; an integer selects one relative depth.",
    )
}

fn tree_rect_schema() -> KindSchema {
    primitive_schema(
        "treemap",
        "tree_rect",
        "Rectangles for visible treemap nodes.",
        RECT_CHANNELS.iter().copied().map(channel),
    )
    .property("node_mode", node_mode_property())
}

fn tree_label_schema() -> KindSchema {
    primitive_schema(
        "treemap",
        "tree_label",
        "Labels fitted inside visible treemap nodes.",
        LABEL_CHANNELS.iter().copied().map(channel),
    )
    .property("node_mode", node_mode_property())
    .property(
        "fit",
        PropertySchema::optional(
            ValueShape::Atom {
                values: [
                    ("ellipsis", "Truncate overflowing labels with an ellipsis."),
                    ("hide", "Hide labels that do not fit."),
                ]
                .into_iter()
                .map(|(value, docs)| EnumValueSchema {
                    value: value.to_string(),
                    docs: docs.to_string(),
                })
                .collect(),
            },
            "Overflow behavior for labels.",
        ),
    )
    .property(
        "padding_px",
        PropertySchema::optional(ValueShape::Number, "Inner label padding in pixels."),
    )
    .property(
        "min_width_px",
        PropertySchema::optional(ValueShape::Number, "Minimum node width for a label."),
    )
    .property(
        "min_height_px",
        PropertySchema::optional(ValueShape::Number, "Minimum node height for a label."),
    )
}

fn tree_header_schema() -> KindSchema {
    primitive_schema(
        "treemap",
        "tree_header",
        "Header bars for visible non-leaf treemap nodes.",
        HEADER_CHANNELS.iter().copied().map(channel),
    )
    .property(
        "min_depth",
        PropertySchema::optional(ValueShape::Integer, "Minimum relative hierarchy depth."),
    )
    .property(
        "max_depth",
        PropertySchema::optional(ValueShape::Integer, "Maximum relative hierarchy depth."),
    )
    .property(
        "padding_px",
        PropertySchema::optional(ValueShape::Number, "Header text padding in pixels."),
    )
}

fn apply_node_mode<M>(
    value: Option<&ResolvedValue>,
    mark: M,
    apply: impl FnOnce(M, TreeRectNodeMode) -> M,
) -> Result<M, NativeLoweringError> {
    let Some(value) = value else {
        return Ok(mark);
    };
    let mode = match value {
        ResolvedValue::String(value) if value == "leaves" => TreeRectNodeMode::VisibleLeaves,
        ResolvedValue::String(value) if value == "all_visible" => TreeRectNodeMode::AllVisible,
        ResolvedValue::Integer(value) => {
            TreeRectNodeMode::Depth((*value).try_into().map_err(|_| {
                NativeLoweringError::InvalidPropertyType {
                    property: "node_mode".to_string(),
                    expected: "non-negative depth".to_string(),
                }
            })?)
        }
        _ => {
            return Err(NativeLoweringError::InvalidPropertyType {
                property: "node_mode".to_string(),
                expected: "`leaves`, `all_visible`, or non-negative depth".to_string(),
            });
        }
    };
    Ok(apply(mark, mode))
}

fn lower_tree_rect(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<avenger_chart_core::PlotMark<Treemap>>, NativeLoweringError> {
    let mut mark = TreeRect::new();
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }
    mark = apply_node_mode(
        declaration.properties.get("node_mode"),
        mark,
        |mark, mode| mark.node_mode(mode),
    )?;
    for (name, value) in &declaration.properties {
        if name != "node_mode" {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
    Ok(mark.into_plot_marks())
}

fn lower_tree_label(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<avenger_chart_core::PlotMark<Treemap>>, NativeLoweringError> {
    let mut mark = TreeLabel::new();
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }
    mark = apply_node_mode(
        declaration.properties.get("node_mode"),
        mark,
        |mark, mode| mark.node_mode(mode),
    )?;
    if let Some(ResolvedValue::String(value)) = declaration.properties.get("fit") {
        mark = mark.fit(match value.as_str() {
            "ellipsis" => TreeLabelFit::Ellipsis,
            "hide" => TreeLabelFit::Hide,
            _ => unreachable!("schema validates label fit"),
        });
    }
    if let Some(ResolvedValue::Number(value)) = declaration.properties.get("padding_px") {
        mark = mark.padding(*value as f32);
    }
    let width = number_property(declaration, "min_width_px")?;
    let height = number_property(declaration, "min_height_px")?;
    if width.is_some() || height.is_some() {
        mark = mark.min_size_px(width.unwrap_or(10.0) as f32, height.unwrap_or(8.0) as f32);
    }
    for (name, value) in &declaration.properties {
        if !matches!(
            name.as_str(),
            "node_mode" | "fit" | "padding_px" | "min_width_px" | "min_height_px"
        ) {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
    Ok(mark.into_plot_marks())
}

fn lower_tree_header(
    declaration: &ResolvedDeclaration,
) -> Result<Vec<avenger_chart_core::PlotMark<Treemap>>, NativeLoweringError> {
    let mut mark = TreeHeader::new();
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }
    let min = integer_property(declaration, "min_depth")?.unwrap_or(1);
    let max = integer_property(declaration, "max_depth")?.unwrap_or(1);
    mark = mark.depth_range(min..=max);
    if let Some(value) = number_property(declaration, "padding_px")? {
        mark = mark.padding(value as f32);
    }
    for (name, value) in &declaration.properties {
        if !matches!(name.as_str(), "min_depth" | "max_depth" | "padding_px") {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
    Ok(mark.into_plot_marks())
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

fn number_property(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<f64>, NativeLoweringError> {
    match declaration.properties.get(name) {
        None => Ok(None),
        Some(ResolvedValue::Number(value)) => Ok(Some(*value)),
        Some(_) => Err(NativeLoweringError::InvalidPropertyType {
            property: name.to_string(),
            expected: "number".to_string(),
        }),
    }
}

fn integer_property(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<usize>, NativeLoweringError> {
    match declaration.properties.get(name) {
        None => Ok(None),
        Some(ResolvedValue::Integer(value)) => {
            (*value)
                .try_into()
                .map(Some)
                .map_err(|_| NativeLoweringError::InvalidPropertyType {
                    property: name.to_string(),
                    expected: "non-negative integer".to_string(),
                })
        }
        Some(_) => Err(NativeLoweringError::InvalidPropertyType {
            property: name.to_string(),
            expected: "integer".to_string(),
        }),
    }
}
