//! Avenger-language registration for geographic coordinates and marks.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use avenger_chart_core::{ChannelValue, ChartTool, CoordinationScope, IntoPlotMark};
use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, NativeOutputValue, ObjectLanguageDefinition,
    ResolvedDeclaration, ResolvedValue,
};
use avenger_chart_marks::{
    Symbol,
    language::{
        apply_common_mark_state, channel, is_common_mark_property, lower_line, lower_rect,
        lower_uniform_raster, primitive_schema, uniform_raster_schema,
    },
};
use avenger_chart_schema::{
    BodyMode, EnumValueSchema, ExportSchema, KindSchema, NativeKindKey, NativeKindNamespace,
    PartSchema, PropertySchema, ValueShape,
};

use crate::{
    Geo, GeoPanZoom, GeoShape, ProjectionKind, RasterTileLayer, TileLoadingPolicy,
    marks::GeoPositionChannels,
};

const LINE_CHANNELS: &[&str] = &[
    "x",
    "y",
    "lon",
    "lat",
    "stroke",
    "stroke_width",
    "stroke_dash",
    "opacity",
    "stroke_cap",
    "stroke_join",
    "defined",
    "order",
];
const RECT_CHANNELS: &[&str] = &[
    "x",
    "x2",
    "y",
    "y2",
    "fill",
    "fill_pattern",
    "stroke",
    "stroke_width",
    "corner_radius",
    "opacity",
];
const SYMBOL_CHANNELS: &[&str] = &[
    "x",
    "y",
    "lon",
    "lat",
    "size",
    "fill",
    "fill_pattern",
    "stroke",
    "stroke_width",
    "shape",
    "angle",
    "opacity",
];
const SHAPE_CHANNELS: &[&str] = &[
    "geometry",
    "x",
    "y",
    "x2",
    "y2",
    "fill",
    "stroke",
    "stroke_width",
    "opacity",
];

pub fn definition() -> CoordinateLanguageDefinition<Geo> {
    CoordinateLanguageDefinition::new("geo", coordinate_schema(), lower_geo)
        .mark(
            "line",
            primitive_schema(
                "geo",
                "line",
                "A projected line with planar or longitude/latitude positions.",
                LINE_CHANNELS.iter().copied().map(channel),
            ),
            lower_line::<Geo>,
        )
        .mark(
            "rect",
            primitive_schema(
                "geo",
                "rect",
                "A rectangle in projected geo plot coordinates.",
                RECT_CHANNELS.iter().copied().map(channel),
            ),
            lower_rect::<Geo>,
        )
        .mark_with_coordinate("symbol", symbol_schema(), lower_geo_symbol)
        .mark_with_coordinate(
            "geo_shape",
            primitive_schema(
                "geo",
                "geo_shape",
                "A GeoJSON or WKB geometry projected through the geo coordinate system.",
                SHAPE_CHANNELS.iter().copied().map(channel),
            ),
            lower_geo_shape,
        )
        .mark(
            "uniform_raster_2d",
            uniform_raster_schema("geo"),
            lower_uniform_raster::<Geo>,
        )
        .tool("geo_pan_zoom", geo_pan_zoom_schema(), lower_geo_pan_zoom)
}

pub fn tile_resource_definition() -> ObjectLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Resource, "tiles"),
        "A reusable XYZ raster tile source for geographic charts.",
    )
    .body_mode(BodyMode::Properties)
    .property(
        "kind",
        PropertySchema::required(
            ValueShape::Atom {
                values: vec![EnumValueSchema {
                    value: "xyz".to_string(),
                    docs: "A `{z}/{x}/{y}` raster tile pyramid.".to_string(),
                }],
            },
            "Tile pyramid kind; v1 supports XYZ tiles.",
        ),
    )
    .property(
        "url",
        PropertySchema::required(
            ValueShape::String,
            "URL template containing `{z}`, `{x}`, and `{y}` placeholders.",
        ),
    )
    .property(
        "tile_size",
        PropertySchema::optional(ValueShape::Integer, "Tile edge length in pixels."),
    )
    .property(
        "min_zoom",
        PropertySchema::optional(ValueShape::Integer, "Minimum available tile zoom."),
    )
    .property(
        "max_zoom",
        PropertySchema::optional(ValueShape::Integer, "Maximum available tile zoom."),
    )
    .property(
        "attribution",
        PropertySchema::optional(ValueShape::String, "Required source attribution text."),
    )
    .property(
        "subdomains",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::String)),
            "Subdomains substituted for `{s}` in deterministic order.",
        ),
    )
    .property(
        "zindex",
        PropertySchema::optional(ValueShape::Integer, "Default layer z-index."),
    )
    .property(
        "loading_policy",
        PropertySchema::optional(
            ValueShape::Atom {
                values: [
                    ("immediate", "Fetch and render the target zoom directly."),
                    (
                        "smooth_zoom",
                        "Use the native fallback and prefetch policy while zooming.",
                    ),
                ]
                .into_iter()
                .map(|(value, docs)| EnumValueSchema {
                    value: value.to_string(),
                    docs: docs.to_string(),
                })
                .collect(),
            },
            "Tile loading and fallback behavior.",
        ),
    );
    ObjectLanguageDefinition {
        schema,
        lowerer: lower_tile_resource,
    }
}

fn coordinate_schema() -> KindSchema {
    KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, "geo"),
        "A geographic map projection with an optional authored viewport.",
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "projection",
        PropertySchema::optional(
            projection_shape(),
            "Map projection; defaults to Equal Earth.",
        ),
    )
    .property(
        "center_lon_lat",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::Number)),
            "Viewport center as `[longitude, latitude]` in degrees.",
        ),
    )
    .property(
        "zoom",
        PropertySchema::optional(ValueShape::Number, "Initial slippy-style zoom level."),
    )
    .property(
        "rotate",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::Number)),
            "Three-axis spherical rotation in degrees.",
        ),
    )
    .property(
        "precision",
        PropertySchema::optional(
            ValueShape::Number,
            "Adaptive projection resampling precision in pixels; zero disables it.",
        ),
    )
    .property(
        "viewport_id",
        PropertySchema::optional(ValueShape::String, "Runtime viewport state id prefix."),
    )
    .property(
        "tiles",
        PropertySchema::optional(
            ValueShape::ConfiguredReference {
                namespaces: BTreeSet::from([NativeKindNamespace::Resource]),
                properties: BTreeMap::from([(
                    "zindex".to_string(),
                    PropertySchema::optional(
                        ValueShape::Integer,
                        "Use-site z-index overriding the resource default.",
                    ),
                )]),
            },
            "Raster tile resource and use-site layer configuration.",
        ),
    )
}

fn projection_shape() -> ValueShape {
    let simple = [
        ("equal_earth", "Equal Earth projection."),
        ("natural_earth", "Natural Earth I projection."),
        ("winkel_tripel", "Winkel Tripel projection."),
        ("equirectangular", "Equirectangular projection."),
        ("mercator", "Web Mercator-compatible projection."),
        ("albers", "CONUS Albers equal-area projection."),
    ];
    let fields = BTreeMap::from([
        (
            "kind".to_string(),
            PropertySchema::required(
                ValueShape::Atom {
                    values: [
                        ("conic_equal_area", "Conic equal-area projection."),
                        ("conic_conformal", "Conic conformal projection."),
                        ("identity", "Planar identity projection."),
                    ]
                    .into_iter()
                    .map(|(value, docs)| EnumValueSchema {
                        value: value.to_string(),
                        docs: docs.to_string(),
                    })
                    .collect(),
                },
                "Structured projection kind.",
            ),
        ),
        (
            "parallels".to_string(),
            PropertySchema::optional(
                ValueShape::Array(Box::new(ValueShape::Number)),
                "Two standard parallels in degrees for conic projections.",
            ),
        ),
        (
            "reflect_y".to_string(),
            PropertySchema::optional(
                ValueShape::Boolean,
                "Flip planar y for identity-projected GIS coordinates.",
            ),
        ),
    ]);
    ValueShape::Union(vec![
        ValueShape::Atom {
            values: simple
                .into_iter()
                .map(|(value, docs)| EnumValueSchema {
                    value: value.to_string(),
                    docs: docs.to_string(),
                })
                .collect(),
        },
        ValueShape::Object(fields),
    ])
}

fn lower_geo(declaration: &ResolvedDeclaration) -> Result<Geo, NativeLoweringError> {
    let mut geo = match declaration.properties.get("projection") {
        None => Geo::new(),
        Some(ResolvedValue::String(kind)) => match kind.as_str() {
            "equal_earth" => Geo::equal_earth(),
            "natural_earth" => Geo::natural_earth(),
            "winkel_tripel" => Geo::winkel_tripel(),
            "equirectangular" => Geo::equirectangular(),
            "mercator" => Geo::mercator(),
            "albers" => Geo::albers_usa_conus(),
            _ => return Err(invalid_projection(declaration, kind)),
        },
        Some(ResolvedValue::Object(fields)) => lower_structured_projection(declaration, fields)?,
        Some(_) => {
            return Err(NativeLoweringError::InvalidPropertyType {
                property: "projection".to_string(),
                expected: "projection atom or object".to_string(),
            });
        }
    };
    if let Some(value) = declaration.properties.get("center_lon_lat") {
        let [lon, lat] = number_pair(value, "center_lon_lat")?;
        geo = geo.center_lon_lat(lon, lat);
    }
    if let Some(ResolvedValue::Number(value)) = declaration.properties.get("zoom") {
        geo = geo.zoom(*value);
    }
    if let Some(value) = declaration.properties.get("rotate") {
        let values = number_array::<3>(value, "rotate")?;
        geo = geo.rotate(values);
    }
    if let Some(ResolvedValue::Number(value)) = declaration.properties.get("precision") {
        geo = geo.precision(*value);
    }
    if let Some(ResolvedValue::String(value)) = declaration.properties.get("viewport_id") {
        geo = geo.viewport_id(value.clone());
    }
    if let Some(value) = declaration.properties.get("tiles") {
        let ResolvedValue::Configured { head, properties } = value else {
            return Err(NativeLoweringError::InvalidPropertyType {
                property: "tiles".to_string(),
                expected: "configured tile resource".to_string(),
            });
        };
        let ResolvedValue::Output(NativeOutputValue::Opaque(resource)) = head.as_ref() else {
            return Err(NativeLoweringError::InvalidPropertyType {
                property: "tiles".to_string(),
                expected: "tile resource reference".to_string(),
            });
        };
        let mut layer = resource
            .downcast_ref::<RasterTileLayer>()
            .ok_or_else(|| NativeLoweringError::InvalidPropertyType {
                property: "tiles".to_string(),
                expected: "XYZ raster tile resource".to_string(),
            })?
            .clone();
        if let Some(ResolvedValue::Integer(zindex)) = properties.get("zindex") {
            layer = layer.zindex(i32::try_from(*zindex).map_err(|_| {
                NativeLoweringError::InvalidPropertyType {
                    property: "tiles.zindex".to_string(),
                    expected: "32-bit integer".to_string(),
                }
            })?);
        }
        geo = geo.tiles(layer);
    }
    Ok(geo)
}

fn lower_tile_resource(
    declaration: &ResolvedDeclaration,
) -> Result<Box<dyn std::any::Any + Send + Sync>, NativeLoweringError> {
    let kind = string_value(declaration, "kind")?;
    if kind != "xyz" {
        return Err(NativeLoweringError::Lowering {
            kind: declaration.kind.clone(),
            message: format!("unsupported tile resource kind `{kind}`"),
        });
    }
    let mut layer = RasterTileLayer::xyz(string_value(declaration, "url")?);
    if let Some(name) = &declaration.source_name {
        layer = layer.id(name.clone());
    }
    if let Some(value) = integer_value(declaration, "tile_size")? {
        layer = layer.tile_size(u32::try_from(value).map_err(|_| {
            NativeLoweringError::InvalidPropertyType {
                property: "tile_size".to_string(),
                expected: "positive 32-bit integer".to_string(),
            }
        })?);
    }
    if let Some(value) = integer_value(declaration, "min_zoom")? {
        layer = layer.min_zoom(u8::try_from(value).map_err(|_| {
            NativeLoweringError::InvalidPropertyType {
                property: "min_zoom".to_string(),
                expected: "zoom from 0 through 255".to_string(),
            }
        })?);
    }
    if let Some(value) = integer_value(declaration, "max_zoom")? {
        layer = layer.max_zoom(u8::try_from(value).map_err(|_| {
            NativeLoweringError::InvalidPropertyType {
                property: "max_zoom".to_string(),
                expected: "zoom from 0 through 255".to_string(),
            }
        })?);
    }
    if let Some(ResolvedValue::String(value)) = declaration.properties.get("attribution") {
        layer = layer.attribution(value.clone());
    }
    if let Some(ResolvedValue::Array(values)) = declaration.properties.get("subdomains") {
        let values = values
            .iter()
            .map(|value| match value {
                ResolvedValue::String(value) => Ok(value.clone()),
                _ => Err(NativeLoweringError::InvalidPropertyType {
                    property: "subdomains".to_string(),
                    expected: "array of strings".to_string(),
                }),
            })
            .collect::<Result<Vec<_>, _>>()?;
        layer = layer.subdomains(values);
    }
    if let Some(value) = integer_value(declaration, "zindex")? {
        layer = layer.zindex(i32::try_from(value).map_err(|_| {
            NativeLoweringError::InvalidPropertyType {
                property: "zindex".to_string(),
                expected: "32-bit integer".to_string(),
            }
        })?);
    }
    if let Some(ResolvedValue::String(policy)) = declaration.properties.get("loading_policy") {
        layer = layer.loading_policy(match policy.as_str() {
            "immediate" => TileLoadingPolicy::Immediate,
            "smooth_zoom" => TileLoadingPolicy::smooth_zoom_default(),
            _ => {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: "loading_policy".to_string(),
                    expected: "immediate or smooth_zoom".to_string(),
                });
            }
        });
    }
    layer.validate()?;
    Ok(Box::new(layer))
}

fn string_value<'a>(
    declaration: &'a ResolvedDeclaration,
    property: &str,
) -> Result<&'a str, NativeLoweringError> {
    match declaration.get(property)? {
        ResolvedValue::String(value) => Ok(value),
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "string".to_string(),
        }),
    }
}

fn integer_value(
    declaration: &ResolvedDeclaration,
    property: &str,
) -> Result<Option<i64>, NativeLoweringError> {
    match declaration.properties.get(property) {
        None => Ok(None),
        Some(ResolvedValue::Integer(value)) => Ok(Some(*value)),
        Some(_) => Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "integer".to_string(),
        }),
    }
}

fn lower_structured_projection(
    declaration: &ResolvedDeclaration,
    fields: &indexmap::IndexMap<String, ResolvedValue>,
) -> Result<Geo, NativeLoweringError> {
    let Some(ResolvedValue::String(kind)) = fields.get("kind") else {
        return Err(NativeLoweringError::InvalidPropertyType {
            property: "projection.kind".to_string(),
            expected: "projection kind".to_string(),
        });
    };
    match kind.as_str() {
        "conic_equal_area" | "conic_conformal" => {
            let parallels =
                fields
                    .get("parallels")
                    .ok_or_else(|| NativeLoweringError::Lowering {
                        kind: declaration.kind.clone(),
                        message: "conic projection requires two `parallels`".to_string(),
                    })?;
            let [first, second] = number_pair(parallels, "projection.parallels")?;
            Ok(if kind == "conic_equal_area" {
                Geo::conic_equal_area((first, second))
            } else {
                Geo::conic_conformal((first, second))
            })
        }
        "identity" => {
            let reflect_y = matches!(fields.get("reflect_y"), Some(ResolvedValue::Boolean(true)));
            Ok(Geo::with_kind(ProjectionKind::Identity { reflect_y }))
        }
        _ => Err(invalid_projection(declaration, kind)),
    }
}

fn invalid_projection(declaration: &ResolvedDeclaration, kind: &str) -> NativeLoweringError {
    NativeLoweringError::Lowering {
        kind: declaration.kind.clone(),
        message: format!("unsupported projection `{kind}`"),
    }
}

fn number_pair(value: &ResolvedValue, property: &str) -> Result<[f64; 2], NativeLoweringError> {
    number_array(value, property)
}

fn number_array<const N: usize>(
    value: &ResolvedValue,
    property: &str,
) -> Result<[f64; N], NativeLoweringError> {
    let ResolvedValue::Array(values) = value else {
        return Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: format!("array of {N} numbers"),
        });
    };
    let numbers = values
        .iter()
        .map(|value| match value {
            ResolvedValue::Number(value) => Ok(*value),
            _ => Err(NativeLoweringError::InvalidPropertyType {
                property: property.to_string(),
                expected: format!("array of {N} numbers"),
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    numbers
        .try_into()
        .map_err(|_| NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: format!("array of {N} numbers"),
        })
}

fn symbol_schema() -> KindSchema {
    primitive_schema(
        "geo",
        "symbol",
        "A symbol positioned by projected x/y or geographic lon/lat channels.",
        SYMBOL_CHANNELS.iter().copied().map(channel),
    )
    .property(
        "lon_lat",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::SqlExpression)),
            "Convenience pair `[longitude, latitude]`; do not also author `lon` or `lat`.",
        ),
    )
}

fn lower_geo_symbol(
    geo: &Geo,
    declaration: &ResolvedDeclaration,
) -> Result<Vec<avenger_chart_core::PlotMark<Geo>>, NativeLoweringError> {
    let mut mark = Symbol::<Geo>::new();
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }

    let has_projected_position =
        declaration.properties.contains_key("x") || declaration.properties.contains_key("y");
    if let Some(value) = declaration.properties.get("lon_lat") {
        let ResolvedValue::Array(values) = value else {
            unreachable!("schema validates lon_lat")
        };
        if values.len() != 2 {
            return Err(NativeLoweringError::InvalidPropertyType {
                property: "lon_lat".to_string(),
                expected: "two channel expressions".to_string(),
            });
        }
        let lon = ordinary_expr("lon_lat", &values[0])?;
        let lat = ordinary_expr("lon_lat", &values[1])?;
        mark = mark.lon_lat(geo, lon, lat);
    } else if !has_projected_position
        && let (Some(lon), Some(lat)) = (
            declaration.properties.get("lon"),
            declaration.properties.get("lat"),
        )
    {
        let lon = ordinary_expr("lon", lon)?;
        let lat = ordinary_expr("lat", lat)?;
        mark = mark.lon_lat(geo, lon, lat);
    }

    for (name, value) in &declaration.properties {
        if is_common_mark_property(name) {
            continue;
        }
        if name == "lon_lat" {
            continue;
        } else if matches!(name.as_str(), "lon" | "lat") {
            if has_projected_position {
                mark = mark.with_channel_value(name, ordinary_channel(name, value)?.no_scale());
            }
        } else if name == "fill_pattern" {
            let value = match value {
                ResolvedValue::Pattern(value) => value.clone(),
                ResolvedValue::Channel(value) => value.channel_value().clone().into(),
                ResolvedValue::Expr(value) => value.clone().into(),
                _ => {
                    return Err(NativeLoweringError::InvalidPropertyType {
                        property: name.clone(),
                        expected: "resolved pattern channel".to_string(),
                    });
                }
            };
            mark = mark.with_pattern_channel_value(name, value);
        } else {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
    apply_common_mark_state::<Geo, _>(&mut mark, declaration)?;
    Ok(mark.into_plot_marks())
}

fn lower_geo_shape(
    geo: &Geo,
    declaration: &ResolvedDeclaration,
) -> Result<Vec<avenger_chart_core::PlotMark<Geo>>, NativeLoweringError> {
    let mut mark = GeoShape::<Geo>::new();
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }
    for (name, value) in &declaration.properties {
        if name == "geometry" {
            mark = mark.geometry(geo, ordinary_expr(name, value)?);
        } else if !is_common_mark_property(name) {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
    apply_common_mark_state::<Geo, _>(&mut mark, declaration)?;
    Ok(mark.into_plot_marks())
}

fn geo_pan_zoom_schema() -> KindSchema {
    let mut schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Tool, "geo_pan_zoom"),
        "Projected-plane pan, wheel zoom, box zoom, and reset behavior for geo plots.",
    )
    .property(
        "viewport_id",
        PropertySchema::optional(ValueShape::Identifier, "Geo viewport state id prefix."),
    )
    .property(
        "sharing",
        PropertySchema::optional(
            ValueShape::CoordinationScope,
            "Sharing scope for viewport parameters.",
        ),
    )
    .property(
        "drag_button",
        PropertySchema::optional(ValueShape::Identifier, "Pointer button used for dragging."),
    )
    .property(
        "scroll_zoom",
        PropertySchema::optional(ValueShape::Boolean, "Enable wheel zoom."),
    )
    .property(
        "zoom_base",
        PropertySchema::optional(ValueShape::Number, "Multiplicative wheel-zoom base."),
    )
    .property(
        "consume_wheel",
        PropertySchema::optional(ValueShape::Boolean, "Consume handled wheel events."),
    )
    .property(
        "box_zoom",
        PropertySchema::optional(ValueShape::Boolean, "Enable drag-box zoom."),
    )
    .property(
        "box_zoom_requires_shift",
        PropertySchema::optional(ValueShape::Boolean, "Require Shift for drag-box zoom."),
    )
    .property(
        "box_zoom_min_size_px",
        PropertySchema::optional(ValueShape::Number, "Minimum accepted box size in pixels."),
    )
    .property(
        "settle_exact",
        PropertySchema::optional(ValueShape::Boolean, "Run exact evaluation after previews."),
    )
    .property(
        "enabled_by_default",
        PropertySchema::optional(ValueShape::Boolean, "Initial enabled state."),
    )
    .part(PartSchema {
        alias: "selection".to_string(),
        runtime_kind: "rect".to_string(),
        runtime_alias: Some("selection".to_string()),
        targetable: true,
        docs: "Visible box-zoom overlay.".to_string(),
    });
    schema.compatible_coordinates.insert("geo".to_string());
    for (alias, kind, docs) in [
        (
            "enabled",
            "param<boolean>",
            "Whether the tool handles input.",
        ),
        ("center_x", "param<float64>", "Projected viewport center x."),
        ("center_y", "param<float64>", "Projected viewport center y."),
        (
            "units_per_pixel",
            "param<float64>",
            "Projected units per display pixel.",
        ),
        ("focus_x", "param<float64>", "Most recent zoom focus x."),
        ("focus_y", "param<float64>", "Most recent zoom focus y."),
        (
            "box_active",
            "param<boolean>",
            "Whether box zoom is active.",
        ),
        ("box_x0", "param<float64>", "Box starting x coordinate."),
        ("box_y0", "param<float64>", "Box starting y coordinate."),
        ("box_x1", "param<float64>", "Box ending x coordinate."),
        ("box_y1", "param<float64>", "Box ending y coordinate."),
    ] {
        schema = schema.export(ExportSchema {
            alias: alias.to_string(),
            value_kind: kind.to_string(),
            lazy: false,
            binding_property: None,
            default_property: None,
            docs: docs.to_string(),
        });
    }
    schema
}

fn lower_geo_pan_zoom(
    declaration: &ResolvedDeclaration,
) -> Result<Arc<dyn ChartTool<Geo>>, NativeLoweringError> {
    let mut tool = GeoPanZoom::new();
    if let Some(name) = &declaration.source_name {
        tool = tool.id(name.clone());
    }
    for (name, value) in &declaration.properties {
        tool = match name.as_str() {
            "viewport_id" => tool.viewport_id(language_string(name, value)?),
            "sharing" => tool.sharing(language_scope(name, value)?),
            "drag_button" => tool.drag_button(language_string(name, value)?),
            "scroll_zoom" => tool.scroll_zoom(language_bool(name, value)?),
            "zoom_base" => tool.zoom_base(language_number(name, value)?),
            "consume_wheel" => tool.consume_wheel(language_bool(name, value)?),
            "box_zoom" => tool.box_zoom(language_bool(name, value)?),
            "box_zoom_requires_shift" => tool.box_zoom_requires_shift(language_bool(name, value)?),
            "box_zoom_min_size_px" => tool.box_zoom_min_size_px(language_number(name, value)?),
            "settle_exact" => tool.settle_exact(language_bool(name, value)?),
            "enabled_by_default" => tool.enabled_by_default(language_bool(name, value)?),
            _ => {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: name.clone(),
                    expected: "a registered geo_pan_zoom property".to_string(),
                });
            }
        };
    }
    Ok(Arc::new(tool))
}

fn language_string<'a>(
    property: &str,
    value: &'a ResolvedValue,
) -> Result<&'a str, NativeLoweringError> {
    let ResolvedValue::String(value) = value else {
        return Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "identifier".to_string(),
        });
    };
    Ok(value)
}

fn language_bool(property: &str, value: &ResolvedValue) -> Result<bool, NativeLoweringError> {
    let ResolvedValue::Boolean(value) = value else {
        return Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "boolean".to_string(),
        });
    };
    Ok(*value)
}

fn language_number(property: &str, value: &ResolvedValue) -> Result<f64, NativeLoweringError> {
    match value {
        ResolvedValue::Number(value) => Ok(*value),
        ResolvedValue::Integer(value) => Ok(*value as f64),
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "number".to_string(),
        }),
    }
}

fn language_scope(
    property: &str,
    value: &ResolvedValue,
) -> Result<CoordinationScope, NativeLoweringError> {
    match value {
        ResolvedValue::String(value) if value == "shared" => Ok(CoordinationScope::Shared),
        ResolvedValue::String(value) if value == "free" => Ok(CoordinationScope::Free),
        ResolvedValue::Integer(level) => {
            (*level)
                .try_into()
                .map(CoordinationScope::Level)
                .map_err(|_| NativeLoweringError::InvalidPropertyType {
                    property: property.to_string(),
                    expected: "scope level from 0 through 255".to_string(),
                })
        }
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: property.to_string(),
            expected: "shared, free, or level".to_string(),
        }),
    }
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

fn ordinary_expr(
    name: &str,
    value: &ResolvedValue,
) -> Result<datafusion::logical_expr::Expr, NativeLoweringError> {
    match value {
        ResolvedValue::Channel(value) => Ok(value.data_expr().clone()),
        ResolvedValue::Expr(value) => Ok(value.clone()),
        ResolvedValue::Output(NativeOutputValue::Expr(value)) => Ok(value.clone()),
        ResolvedValue::Output(NativeOutputValue::Channel(value)) => Ok(value.data_expr().clone()),
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: name.to_string(),
            expected: "resolved channel expression".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use avenger_chart_core::{ChannelValue, PlotMarkKind};
    use datafusion::logical_expr::{col, lit};

    use super::*;

    #[test]
    fn geo_symbol_lon_lat_lowers_projected_and_spherical_channels_together() {
        let geo = Geo::mercator().center_lon_lat(-73.9857, 40.7484).zoom(10.0);
        let declarations = [
            ResolvedDeclaration::new("symbol").property(
                "lon_lat",
                ResolvedValue::Array(vec![
                    ResolvedValue::Expr(lit(-73.9857)),
                    ResolvedValue::Expr(lit(40.7484)),
                ]),
            ),
            ResolvedDeclaration::new("symbol")
                .property("lon", ResolvedValue::Expr(col("longitude")))
                .property("lat", ResolvedValue::Expr(col("latitude"))),
        ];

        for declaration in declarations {
            let marks = lower_geo_symbol(&geo, &declaration).expect("lower geo symbol");
            let PlotMarkKind::Primitive(mark) = marks[0].kind() else {
                panic!("geo symbol should lower to a primitive mark");
            };
            let channels = mark.data_context().channels();
            assert!(matches!(
                channels.get("x"),
                Some(ChannelValue::Scaled { .. })
            ));
            assert!(matches!(
                channels.get("y"),
                Some(ChannelValue::Scaled { .. })
            ));
            assert!(matches!(
                channels.get("lon"),
                Some(ChannelValue::Value { .. })
            ));
            assert!(matches!(
                channels.get("lat"),
                Some(ChannelValue::Value { .. })
            ));
        }
    }
}
