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
    ("Axis", &["classes", "id", "route", "value_step", "values"]),
    ("NestedAxisLevel", &["title"]),
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
    ("Fragment", &["version", "type", "message", "values"]),
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
    ("Title", &["text", "syntax"]),
    ("Subtitle", &["text"]),
    (
        "Concat",
        &[
            "id",
            "direction",
            "children",
            "resolve",
            "tracks",
            "parts",
            "horizontal_distribute",
            "vertical_distribute",
            "title",
            "partition",
            "facet",
            "classes",
        ],
    ),
    (
        "Wrap",
        &[
            "id",
            "children",
            "resolve",
            "row_height",
            "parts",
            "distribute",
            "title",
            "partition",
            "facet",
            "classes",
        ],
    ),
    ("Track", &["length", "share", "max"]),
    ("Facet", &["column", "display", "classes", "id"]),
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
            "axis",
            "legend",
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
    // Whether a scale draws its axis or legend, and the legend's type, decide which nodes the
    // style tree holds, as whether a geo plot draws its graticule and its sphere does.
    ("Axis", &["display"]),
    (
        "PartRules",
        &[
            "axis_ticks",
            "axis_labels",
            "axis_title",
            "legend",
            "facet_labels",
            "facet_title",
            "attribution",
            "widget",
        ],
    ),
    ("Labels", &["format", "markup", "frame", "sides"]),
    ("ValueStep", &["step", "interval", "start"]),
    (
        "Legend",
        &["display", "type", "value_step", "values", "classes", "id"],
    ),
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
            "orient",
            "x",
            "y",
            "color",
            "stroke",
            "opacity",
            "stroke_opacity",
            "stroke_width",
            "stroke_dasharray",
            "detail",
            "order",
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
            "stroke_dasharray",
            "pattern",
            "detail",
            "order",
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
            "stroke_dasharray",
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
            "projection",
            "family",
            "navigation",
            "graticule",
            "sphere",
            "tiles",
        ],
    ),
    ("GeoNavigation", &["pan_mode", "min_zoom", "max_zoom"]),
    (
        "Projection",
        &[
            "builtin",
            "custom",
            "parallels",
            "parallel",
            "coefficient",
            "reflect_x",
            "reflect_y",
            "rotate",
            "center",
            "clip",
            "precision",
            "data",
            "whole_sphere",
            "box",
        ],
    ),
    ("CustomProjection", &["name", "options"]),
    ("FitBox", &["bounds"]),
    ("SphericalClip", &["antimeridian", "circle_angle", "region"]),
    ("RegionClip", &["polygon", "frame"]),
    (
        "Graticule",
        &[
            "display",
            "step",
            "extent",
            "step_major",
            "extent_major",
            "precision",
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
            "stroke_dasharray",
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
            "stroke_dasharray",
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
            "orient",
            "route",
            "theta",
            "radius",
            "color",
            "stroke",
            "opacity",
            "stroke_opacity",
            "stroke_width",
            "stroke_dasharray",
            "detail",
            "order",
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
            "stroke_dasharray",
            "pattern",
            "detail",
            "order",
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
            "stroke_dasharray",
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
    (
        "Resolve",
        &[
            "x",
            "y",
            "theta",
            "radius",
            "color",
            "fill",
            "stroke",
            "opacity",
            "fill_opacity",
            "stroke_opacity",
            "stroke_width",
            "stroke_dasharray",
            "size",
            "shape",
            "angle",
            "pattern",
            "projection",
            "independent_widgets",
        ],
    ),
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
            "explicit",
            "summary",
            "selection",
            "zero",
            "enabled",
            "nice_count",
            "pin",
            "domain_empty",
            "direction",
            "preferred",
            "reverse",
            "family",
            "navigation",
        ],
    ),
    ("NavigationLimits", &["data", "extent", "min_span"]),
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
    ("CustomScale", &["name", "options"]),
    (
        "SelectionDomain",
        &["selection", "channel", "explicit", "summary"],
    ),
    ("DomainPin", &["value", "position"]),
    ("Selection", &["name", "resolution", "initial"]),
    ("Clause", &["owner", "conditions", "partition", "widget"]),
    (
        "ChartChange",
        &["domains", "selections", "modes", "settled"],
    ),
    ("ScaleDomain", &["scale", "domain"]),
    ("ScaleKey", &["channel", "family", "plot", "position"]),
    ("PlotKey", &["given", "path", "partition"]),
    ("SelectionClauses", &["name", "clauses"]),
    ("ModesOn", &["names"]),
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
    ("Selected", &["selection", "empty", "exclude_own"]),
    ("Tool", &["navigate", "tooltip", "select"]),
    ("LayerTool", &["tooltip", "select"]),
    ("Mode", &["name", "label", "group", "on"]),
    ("Navigate", &["channels", "modes"]),
    (
        "Tooltip",
        &[
            "nearest",
            "max_radius",
            "metric",
            "title",
            "fields",
            "anchor",
            "hide_swatch",
            "modes",
        ],
    ),
    ("TooltipTitle", &["column", "format", "markup"]),
    ("TooltipField", &["column", "title", "format", "markup"]),
    (
        "Select",
        &[
            "selection",
            "interval",
            "point",
            "lasso",
            "scope",
            "modes",
            "throttle",
        ],
    ),
    ("IntervalSelect", &["channels", "cell"]),
    ("PointSelect", &["keys", "toggle"]),
    ("PointKey", &["column", "source"]),
    ("LassoSelect", &["keys"]),
    ("ValueRef", &["index"]),
    ("TableRef", &["index"]),
];

/// The fields of messages that the evaluator maps, which only a later evaluator PR draws.
const DEFERRED_FIELDS: &[(&str, &[&str])] = &[];

/// The messages that the test does not classify field by field: the widgets, whose style fields are
/// their `Title` and `Labels`, which the evaluator maps as those messages.
const DEFERRED: &[(&str, &[&str])] = &[(
    "PR 7, widgets",
    &[
        "Widget",
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
    // The fields of `STYLE_FIELDS` by message, without the elements that each message styles.
    let style_fields: Vec<(&str, &[&str])> = (avenger_chart_eval::STYLE_FIELDS.iter())
        .map(|(message, _, fields)| (*message, *fields))
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
            let mapped = listed(&style_fields, message, field);
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
    for (message, names) in (style_fields.iter()).chain(CONTENT).chain(DEFERRED_FIELDS) {
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
