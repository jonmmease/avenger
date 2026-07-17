//! Avenger-language registration for geographic coordinates and marks.

use std::collections::BTreeMap;

use avenger_chart_core::{ChannelValue, IntoPlotMark};
use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, ResolvedDeclaration, ResolvedValue,
};
use avenger_chart_marks::{
    Symbol,
    language::{
        apply_common_mark_state, channel, is_common_mark_property, lower_line, lower_rect,
        lower_uniform_raster, primitive_schema, uniform_raster_schema,
    },
};
use avenger_chart_schema::{
    BodyMode, EnumValueSchema, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema,
    ValueShape,
};

use crate::{Geo, GeoShape, ProjectionKind};

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
        .mark("symbol", symbol_schema(), lower_geo_symbol)
        .mark(
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
    Ok(geo)
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
    declaration: &ResolvedDeclaration,
) -> Result<Vec<avenger_chart_core::PlotMark<Geo>>, NativeLoweringError> {
    let mut mark = Symbol::<Geo>::new();
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }
    for (name, value) in &declaration.properties {
        if is_common_mark_property(name) {
            continue;
        }
        if name == "lon_lat" {
            let ResolvedValue::Array(values) = value else {
                unreachable!("schema validates lon_lat")
            };
            if values.len() != 2 {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: name.clone(),
                    expected: "two channel expressions".to_string(),
                });
            }
            mark = mark
                .with_channel_value("lon", ordinary_channel(name, &values[0])?)
                .with_channel_value("lat", ordinary_channel(name, &values[1])?);
        } else if name == "fill_pattern" {
            let value = match value {
                ResolvedValue::Pattern(value) => value.clone(),
                ResolvedValue::Channel(value) => value.as_ref().clone().into(),
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
    declaration: &ResolvedDeclaration,
) -> Result<Vec<avenger_chart_core::PlotMark<Geo>>, NativeLoweringError> {
    let mut mark = GeoShape::<Geo>::new();
    if let Some(id) = &declaration.source_name {
        mark = mark.id(id.clone());
    }
    for (name, value) in &declaration.properties {
        if !is_common_mark_property(name) {
            mark = mark.with_channel_value(name, ordinary_channel(name, value)?);
        }
    }
    apply_common_mark_state::<Geo, _>(&mut mark, declaration)?;
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
