//! Every field of the format is a style field that the evaluator maps to a property, a content
//! field, or a field of a message that a later evaluator PR maps, so a new style field cannot reach
//! the evaluator without its property.

use std::collections::{BTreeMap, BTreeSet};

use prost::Message;

/// The fields that are never properties, by message, as the style proposal's Appendix B lists
/// them: texts, formats, values, domains, scale types and options, structure, tools, and
/// selections.
const CONTENT: &[(&str, &[&str])] = &[
    ("Attribution", &["credits"]),
    ("Axis", &["classes", "id", "route"]),
    ("NestedAxisLevel", &["title", "format"]),
    (
        "Chart",
        &[
            "version",
            "tables",
            "root",
            "selections",
            "values",
            "number_format_syntax",
            "datetime_format_syntax",
            "width",
            "height",
            "modes",
        ],
    ),
    ("Table", &["padding", "ipc"]),
    ("Node", &["plot", "concat", "wrap", "empty"]),
    (
        "Plot",
        &[
            "id",
            "title",
            "attribution",
            "tools",
            "partition",
            "resolve",
            "cartesian",
            "polar",
            "geo",
            "classes",
        ],
    ),
    ("Title", &["text", "syntax", "subtitle"]),
    (
        "Concat",
        &[
            "id",
            "direction",
            "children",
            "resolve",
            "layout",
            "title",
            "projection",
            "partition",
            "facet",
            "classes",
        ],
    ),
    (
        "ConcatLayout",
        &[
            "tracks",
            "part_rules",
            "horizontal_distribute",
            "vertical_distribute",
        ],
    ),
    (
        "Wrap",
        &[
            "id",
            "children",
            "resolve",
            "layout",
            "title",
            "projection",
            "partition",
            "facet",
            "classes",
        ],
    ),
    ("WrapLayout", &["row", "part_rules", "distribute"]),
    ("Track", &["length", "share"]),
    ("Share", &["weight", "max"]),
    ("Facet", &["column", "show", "classes", "id"]),
    ("Empty", &["partition"]),
    (
        "Encoding",
        &[
            "direct_field",
            "scaled_field",
            "direct_value",
            "scaled_value",
            "scale",
            "conditional",
            "selection_field",
            "title",
            "band_position",
            "band_level",
        ],
    ),
    (
        "ConditionalValue",
        &[
            "predicate",
            "direct_field",
            "scaled_field",
            "direct_value",
            "scaled_value",
        ],
    ),
    // Whether a scale draws its axis or legend, and the legend's form, decide which nodes the
    // style tree holds.
    ("Axis", &["show"]),
    ("PartRule", &["part", "rule"]),
    ("Labels", &["format", "markup"]),
    ("Ticks", &["interval", "values"]),
    ("TickInterval", &["numeric", "calendar"]),
    ("NumericTicks", &["start", "step"]),
    (
        "CalendarTicks",
        &["start", "months", "days", "milliseconds"],
    ),
    ("Legend", &["show", "form", "values", "classes", "id"]),
    ("LegendWrap", &["flow", "grid"]),
    (
        "SymbolMark",
        &[
            "x",
            "y",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "size",
            "shape",
            "angle",
            "pattern",
        ],
    ),
    (
        "LineMark",
        &[
            "x",
            "y",
            "color",
            "stroke",
            "opacity",
            "stroke_opacity",
            "stroke_width",
            "stroke_dash",
            "detail",
        ],
    ),
    (
        "TrailMark",
        &["x", "y", "color", "stroke", "opacity", "size", "detail"],
    ),
    (
        "AreaMark",
        &[
            "orient",
            "x",
            "y",
            "x2",
            "y2",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "stroke_dash",
            "pattern",
            "detail",
        ],
    ),
    (
        "BarMark",
        &[
            "orient",
            "x",
            "y",
            "x2",
            "y2",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "pattern",
        ],
    ),
    (
        "RectMark",
        &[
            "x",
            "y",
            "x2",
            "y2",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "pattern",
        ],
    ),
    (
        "RuleMark",
        &[
            "x",
            "y",
            "x2",
            "y2",
            "color",
            "stroke",
            "opacity",
            "stroke_width",
            "stroke_dash",
        ],
    ),
    (
        "TextMark",
        &[
            "format",
            "syntax",
            "x",
            "y",
            "text",
            "color",
            "fill",
            "opacity",
            "size",
            "angle",
            "font_weight",
            "font_style",
            "width",
        ],
    ),
    (
        "ImageMark",
        &["x", "y", "x2", "y2", "image", "width", "height"],
    ),
    (
        "PathMark",
        &[
            "x",
            "y",
            "path",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "angle",
            "pattern",
        ],
    ),
    ("NamedOption", &["name", "value"]),
    (
        "OptionValue",
        &["boolean", "float", "integer", "string", "floats", "strings"],
    ),
    ("FloatList", &["values"]),
    ("StringList", &["values"]),
    ("PartitionValue", &["column", "value"]),
    ("CartesianPlot", &["options", "layers"]),
    ("GeoPlot", &["options", "layers"]),
    (
        "GeoLayer",
        &[
            "id",
            "table",
            "tools",
            "partition",
            "symbol",
            "line",
            "trail",
            "rule",
            "text",
            "image",
            "path",
            "shape",
            "classes",
            "widgets",
        ],
    ),
    (
        "Geo",
        &[
            "spherical",
            "planar",
            "family",
            "aspect_ratio",
            "free_aspect",
            "navigation",
        ],
    ),
    ("GeoNavigation", &["pan_mode", "min_zoom", "max_zoom"]),
    (
        "Spherical",
        &[
            "equal_earth",
            "natural_earth1",
            "robinson",
            "mollweide",
            "eckert4",
            "winkel_tripel",
            "aitoff",
            "sinusoidal",
            "equirectangular",
            "mercator",
            "transverse_mercator",
            "miller",
            "orthographic",
            "stereographic",
            "gnomonic",
            "azimuthal_equal_area",
            "azimuthal_equidistant",
            "conic_conformal",
            "conic_equal_area",
            "conic_equidistant",
            "cylindrical_equal_area",
            "hammer",
            "custom",
            "rotate",
            "center",
            "clip",
            "precision",
            "fit_data",
            "fit_sphere",
            "fit_box",
            "graticule",
            "sphere",
            "tiles",
        ],
    ),
    ("ConicProjection", &["parallel1", "parallel2"]),
    ("CylindricalEqualAreaProjection", &["parallel"]),
    ("HammerProjection", &["coefficient"]),
    ("CustomProjection", &["name", "parameters"]),
    ("Planar", &["reflect_y"]),
    ("Rotation", &["lambda", "phi", "gamma"]),
    ("LonLat", &["longitude", "latitude"]),
    ("LonLatBox", &["west", "south", "east", "north"]),
    ("SphericalClip", &["antimeridian", "circle_angle", "region"]),
    ("RegionClip", &["polygon", "frame"]),
    (
        "Graticule",
        &[
            "longitude_step",
            "latitude_step",
            "extent",
            "major_longitude_step",
            "major_latitude_step",
            "major_extent",
            "precision",
            "frame",
            "label_sides",
            "format",
        ],
    ),
    ("LabelSides", &["sides"]),
    (
        "TileLayer",
        &["url_template", "min_zoom", "max_zoom", "tile_size"],
    ),
    (
        "GeoSymbolMark",
        &[
            "longitude",
            "latitude",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "size",
            "shape",
            "angle",
            "pattern",
        ],
    ),
    (
        "GeoLineMark",
        &[
            "route",
            "longitude",
            "latitude",
            "color",
            "stroke",
            "opacity",
            "stroke_opacity",
            "stroke_width",
            "stroke_dash",
            "detail",
        ],
    ),
    (
        "GeoTrailMark",
        &[
            "route",
            "longitude",
            "latitude",
            "color",
            "stroke",
            "opacity",
            "size",
            "detail",
        ],
    ),
    (
        "GeoRuleMark",
        &[
            "route",
            "longitude",
            "latitude",
            "longitude2",
            "latitude2",
            "color",
            "stroke",
            "opacity",
            "stroke_width",
            "stroke_dash",
        ],
    ),
    (
        "GeoTextMark",
        &[
            "format",
            "syntax",
            "longitude",
            "latitude",
            "text",
            "color",
            "fill",
            "opacity",
            "size",
            "angle",
            "font_weight",
            "font_style",
            "width",
        ],
    ),
    (
        "GeoImageMark",
        &["longitude", "latitude", "image", "width", "height"],
    ),
    (
        "GeoPathMark",
        &[
            "longitude",
            "latitude",
            "path",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "angle",
            "pattern",
        ],
    ),
    (
        "GeoShapeMark",
        &[
            "geometry",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "size",
            "shape",
            "pattern",
        ],
    ),
    ("PolarPlot", &["options", "layers"]),
    (
        "PolarLayer",
        &[
            "id",
            "table",
            "tools",
            "partition",
            "symbol",
            "line",
            "trail",
            "area",
            "bar",
            "rect",
            "rule",
            "text",
            "classes",
            "widgets",
        ],
    ),
    ("Polar", &["start_angle", "end_angle", "inner_radius"]),
    (
        "PolarSymbolMark",
        &[
            "theta",
            "radius",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "size",
            "shape",
            "angle",
            "pattern",
        ],
    ),
    (
        "PolarLineMark",
        &[
            "route",
            "theta",
            "radius",
            "color",
            "stroke",
            "opacity",
            "stroke_opacity",
            "stroke_width",
            "stroke_dash",
            "detail",
        ],
    ),
    (
        "PolarTrailMark",
        &[
            "route", "theta", "radius", "color", "stroke", "opacity", "size", "detail",
        ],
    ),
    (
        "PolarAreaMark",
        &[
            "orient",
            "route",
            "theta",
            "radius",
            "theta2",
            "radius2",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "stroke_dash",
            "pattern",
            "detail",
        ],
    ),
    (
        "PolarBarMark",
        &[
            "orient",
            "route",
            "theta",
            "radius",
            "theta2",
            "radius2",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "pattern",
        ],
    ),
    (
        "PolarRectMark",
        &[
            "route",
            "theta",
            "radius",
            "theta2",
            "radius2",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "pattern",
        ],
    ),
    (
        "PolarRuleMark",
        &[
            "route",
            "theta",
            "radius",
            "theta2",
            "radius2",
            "color",
            "stroke",
            "opacity",
            "stroke_width",
            "stroke_dash",
        ],
    ),
    (
        "PolarTextMark",
        &[
            "format",
            "syntax",
            "theta",
            "radius",
            "text",
            "color",
            "fill",
            "opacity",
            "size",
            "angle",
            "font_weight",
            "font_style",
            "width",
        ],
    ),
    (
        "CartesianLayer",
        &[
            "id",
            "table",
            "tools",
            "partition",
            "symbol",
            "line",
            "trail",
            "area",
            "bar",
            "rect",
            "rule",
            "text",
            "image",
            "path",
            "classes",
            "widgets",
        ],
    ),
    ("Resolve", &["channel", "widget", "sharing"]),
    (
        "Scale",
        &[
            "linear",
            "log",
            "pow",
            "symlog",
            "time",
            "band",
            "point",
            "nested_band",
            "ordinal",
            "quantile",
            "quantize",
            "threshold",
            "custom",
            "domain",
            "range",
            "family",
            "axis",
            "legend",
            "navigation_limits",
        ],
    ),
    ("NavigationLimits", &["data", "interval", "min_span"]),
    ("Interval", &["low", "high"]),
    ("LinearScale", &["clamp", "round"]),
    ("LogScale", &["base", "clamp", "round"]),
    ("PowScale", &["exponent", "clamp", "round"]),
    ("SymlogScale", &["constant", "clamp", "round"]),
    ("TimeScale", &["timezone"]),
    (
        "BandScale",
        &[
            "align",
            "round",
            "clip_padding_lower",
            "clip_padding_upper",
            "kind",
        ],
    ),
    (
        "PointScale",
        &[
            "align",
            "round",
            "clip_padding_lower",
            "clip_padding_upper",
            "kind",
        ],
    ),
    (
        "NestedBandScale",
        &[
            "align",
            "nest_scopes",
            "round",
            "clip_padding_lower",
            "clip_padding_upper",
            "kind",
        ],
    ),
    ("OrdinalScale", &["kind"]),
    ("CustomScale", &["type", "options"]),
    ("Domain", &["explicit", "summary", "data", "selection"]),
    ("SummaryDomain", &["value", "fit"]),
    ("DataDomain", &["fit"]),
    (
        "SelectionDomain",
        &["selection", "channel", "explicit", "summary", "data"],
    ),
    ("DomainFit", &["interval", "categories"]),
    (
        "IntervalFit",
        &["zero", "nice", "nice_count", "empty", "low", "mid", "high"],
    ),
    ("CategoryFit", &["sort", "preferred"]),
    ("Selection", &["name", "resolution", "initial"]),
    ("Clause", &["owner", "conditions", "partition", "widget"]),
    ("Condition", &["set", "range", "polygon", "cyclic_range"]),
    ("ValueSet", &["columns", "complement"]),
    ("SetColumn", &["column", "channel", "values"]),
    (
        "ValueRange",
        &["column", "channel", "low", "high", "exclude_end"],
    ),
    ("Polygon", &["x_column", "y_column", "vertices"]),
    (
        "CyclicRange",
        &["column", "channel", "start", "end", "period"],
    ),
    ("Predicate", &["selected", "all", "any", "negate"]),
    ("PredicateList", &["predicates"]),
    (
        "Selected",
        &["selection", "empty_selects_none", "exclude_own"],
    ),
    ("Tool", &["navigate", "tooltip", "select", "modes"]),
    ("LayerTool", &["tooltip", "select", "modes"]),
    ("Mode", &["name", "label", "group", "on"]),
    ("Navigate", &["channels"]),
    (
        "Tooltip",
        &[
            "exact",
            "nearest",
            "title",
            "fields",
            "anchor",
            "hide_swatch",
        ],
    ),
    ("NearestFind", &["max_radius", "metric"]),
    ("TooltipField", &["column", "title", "labels"]),
    (
        "Select",
        &["selection", "interval", "points", "lasso", "scope"],
    ),
    ("IntervalSelect", &["channels", "cell"]),
    ("PointSelect", &["keys", "toggle", "sources"]),
    ("LumpSource", &["key", "source"]),
    ("LassoSelect", &["keys"]),
    ("ValueRef", &["index"]),
    ("TableRef", &["index"]),
];

/// The fields of messages that the evaluator maps, which only a later evaluator PR draws.
const DEFERRED_FIELDS: &[(&str, &[&str])] = &[];

/// The messages that a later evaluator PR maps, whose nodes PR 1 does not build.
const DEFERRED: &[(&str, &[&str])] = &[(
    "PR 7, widgets",
    &[
        "Widget",
        "WidgetTarget",
        "SliderWidget",
        "RangeSliderWidget",
        "RadioWidget",
        "CheckboxesWidget",
        "CheckboxWidget",
        "ModeWidget",
        "WidgetDomain",
        "WidgetSummaryDomain",
        "WidgetDataDomain",
    ],
)];

#[test]
fn every_field_is_mapped_content_or_deferred() {
    let set =
        prost_types::FileDescriptorSet::decode(avenger_chart::proto::FILE_DESCRIPTOR_SET).unwrap();
    let mut fields: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in set
        .file
        .iter()
        .filter(|file| file.package() == "avenger.chart.v1")
    {
        for message in &file.message_type {
            fields
                .entry(message.name().to_string())
                .or_default()
                .extend(message.field.iter().map(|field| field.name().to_string()));
        }
    }
    let deferred: BTreeSet<&str> = DEFERRED
        .iter()
        .flat_map(|(_, messages)| messages.iter().copied())
        .collect();
    let listed = |table: &[(&str, &[&str])], message: &str, field: &str| {
        table
            .iter()
            .any(|(name, fields)| *name == message && fields.contains(&field))
    };
    let mut unlisted = Vec::new();
    for (message, names) in &fields {
        if deferred.contains(message.as_str()) {
            continue;
        }
        for field in names {
            let mapped = listed(avenger_chart_eval::STYLE_FIELDS, message, field);
            let content = listed(CONTENT, message, field);
            assert!(
                !(mapped && content),
                "`{message}.{field}` is both a style field and content"
            );
            if !mapped && !content && !listed(DEFERRED_FIELDS, message, field) {
                unlisted.push(format!("{message}.{field}"));
            }
        }
    }
    assert!(
        unlisted.is_empty(),
        "fields that are neither mapped nor content: {unlisted:?}"
    );
    // Every listed field exists, so a renamed field cannot hide behind a stale entry.
    for (message, names) in (avenger_chart_eval::STYLE_FIELDS.iter())
        .chain(CONTENT)
        .chain(DEFERRED_FIELDS)
    {
        for name in *names {
            assert!(
                fields
                    .get(*message)
                    .is_some_and(|fields| fields.contains(*name)),
                "`{message}.{name}` is not a field of the format"
            );
        }
    }
    for message in deferred {
        assert!(
            fields.contains_key(message),
            "`{message}` is not a message of the format"
        );
    }
}
