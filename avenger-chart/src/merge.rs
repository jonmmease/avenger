//! The merge of the scale specs of a shared scale by the agreement rule of `ScaleSpec`: the specs
//! of the units that share a scale must agree, and the merged spec takes each option, and each
//! field of the guide at every depth, that one of them sets.
//!
//! The functions destructure each guide message completely, so a field that the format adds
//! fails to compile until its merge is written.

use avenger_scales::scales::{band::BandScale, ScaleImpl};

use crate::proto::{self, axis, domain, guide, legend_wrap};

/// Why two specs disagree, as the end of a sentence about them, such as "set `range` to
/// different values".
pub(crate) type Disagreement = String;

type Merged = std::result::Result<(), Disagreement>;

/// Whether the values at two indices of `Chart.values` are equal.
pub(crate) type SameValue<'a> = dyn Fn(u32, u32) -> bool + 'a;

fn differ(field: &str) -> Disagreement {
    format!("set `{field}` to different values")
}

/// Merges `other` into `merged`. The types of the specs must be equal, or a band and a point
/// scale merge into a band scale. Their explicit and
/// selection domains must be equal, and summary and data domains merge, so `merged` keeps its
/// domain. The range, each field of the fit, each option, each field of the guide, and the
/// navigation that one spec sets is the merged value, and one that both set must be equal.
pub(crate) fn merge(
    merged: &mut proto::ScaleSpec,
    other: &proto::ScaleSpec,
    same: &SameValue,
) -> Merged {
    let proto::ScaleSpec {
        r#type,
        options,
        domain,
        range,
        family: _,
        guide,
        fit,
        navigation,
    } = other;
    merge_type(merged, r#type, options)?;
    merge_domain(merged.domain.as_ref(), domain.as_ref(), same)?;
    merge_range(&mut merged.range, range, same)?;
    message(&mut merged.fit, fit, |merged, other| {
        merge_fit(merged, other, same)
    })?;
    merge_options(&mut merged.options, options)?;
    message(&mut merged.navigation, navigation, |merged, other| {
        if merged == other {
            Ok(())
        } else {
            Err("set two navigations of the scale".to_string())
        }
    })?;
    merge_guide(&mut merged.guide, guide.as_ref(), same)
}

/// The type of both specs: their type when it is equal, or a band scale for a band and a point
/// scale, as Vega-Lite merges them, so bars and a line share their categories, and the line's
/// points stand at the middles of the bands. The options of the point scale must be options of a
/// band scale.
fn merge_type(
    merged: &mut proto::ScaleSpec,
    r#type: &str,
    options: &[proto::NamedOption],
) -> Merged {
    match (merged.r#type.as_str(), r#type) {
        (a, b) if a == b => Ok(()),
        ("band", "point") | ("point", "band") => {
            let point = if merged.r#type == "point" {
                &merged.options
            } else {
                options
            };
            let band = BandScale.option_definitions();
            for option in point {
                if !band.iter().any(|definition| definition.name == option.name) {
                    return Err(format!(
                        "set the point option `{}`, which a band scale does not take",
                        option.name
                    ));
                }
            }
            merged.r#type = "band".to_string();
            Ok(())
        }
        _ => Err(differ("type")),
    }
}

/// The range, one value: its values or its palette, and whether it runs in reverse.
fn merge_range(
    merged: &mut Option<proto::Range>,
    other: &Option<proto::Range>,
    same: &SameValue,
) -> Merged {
    use proto::range::Range;
    message(merged, other, |merged, other| {
        let equal = merged.reverse == other.reverse
            && match (&merged.range, &other.range) {
                (Some(Range::Values(a)), Some(Range::Values(b))) => same(*a, *b),
                (a, b) => a == b,
            };
        if equal {
            Ok(())
        } else {
            Err(differ("range"))
        }
    })
}

fn merge_fit(merged: &mut proto::DomainFit, other: &proto::DomainFit, same: &SameValue) -> Merged {
    use proto::{category_fit::Order, domain_fit::Fit};
    match (&mut merged.fit, &other.fit) {
        (_, None) => Ok(()),
        (None, Some(fit)) => {
            merged.fit = Some(*fit);
            Ok(())
        }
        (Some(Fit::Interval(merged)), Some(Fit::Interval(other))) => {
            let proto::IntervalFit { zero, nice, empty } = other;
            value(&mut merged.zero, zero, "fit.interval.zero")?;
            value(&mut merged.nice, nice, "fit.interval.nice")?;
            index(&mut merged.empty, *empty, "fit.interval.empty", same)
        }
        (Some(Fit::Categories(merged)), Some(Fit::Categories(other))) => {
            let proto::CategoryFit { order } = other;
            match (&merged.order, order) {
                (_, None) => Ok(()),
                (None, Some(order)) => {
                    merged.order = Some(*order);
                    Ok(())
                }
                (Some(Order::Preferred(a)), Some(Order::Preferred(b))) if same(*a, *b) => Ok(()),
                (Some(a), Some(b)) if a == b => Ok(()),
                _ => Err(differ("fit.categories.order")),
            }
        }
        _ => Err(differ("fit")),
    }
}

/// Whether a domain is inferred: the producer's summary or the unit's data.
pub(crate) fn inferred(domain: Option<&proto::Domain>) -> bool {
    matches!(
        domain.and_then(|domain| domain.domain.as_ref()),
        None | Some(domain::Domain::Summary(_) | domain::Domain::Data(_))
    )
}

fn domain_equal(a: Option<&proto::Domain>, b: Option<&proto::Domain>, same: &SameValue) -> bool {
    use domain::Domain;
    match (
        a.and_then(|domain| domain.domain.as_ref()),
        b.and_then(|domain| domain.domain.as_ref()),
    ) {
        (Some(Domain::Explicit(a)), Some(Domain::Explicit(b)))
        | (Some(Domain::Summary(a)), Some(Domain::Summary(b))) => same(*a, *b),
        (Some(Domain::Selection(a)), Some(Domain::Selection(b))) => {
            a.selection == b.selection
                && a.channel == b.channel
                && domain_equal(a.fallback.as_deref(), b.fallback.as_deref(), same)
        }
        (a, b) => a == b,
    }
}

fn merge_domain(
    merged: Option<&proto::Domain>,
    other: Option<&proto::Domain>,
    same: &SameValue,
) -> Merged {
    match (inferred(merged), inferred(other)) {
        (true, true) => Ok(()),
        (false, false) if domain_equal(merged, other, same) => Ok(()),
        (false, false) => Err(differ("domain")),
        _ => {
            Err("set an explicit or a selection domain and a summary or a data domain".to_string())
        }
    }
}

/// The options of both specs, in the order of their names, where the specs that set one option
/// set it to equal values.
fn merge_options(merged: &mut Vec<proto::NamedOption>, other: &[proto::NamedOption]) -> Merged {
    for option in other {
        match merged.binary_search_by(|known| known.name.cmp(&option.name)) {
            Ok(index) if merged[index].value == option.value => {}
            Ok(_) => return Err(differ(&format!("options.{}", option.name))),
            Err(index) => merged.insert(index, option.clone()),
        }
    }
    Ok(())
}

fn merge_guide(
    merged: &mut Option<proto::Guide>,
    other: Option<&proto::Guide>,
    same: &SameValue,
) -> Merged {
    use guide::Guide;
    let Some(other) = other.and_then(|guide| guide.guide.as_ref()) else {
        return Ok(());
    };
    let Some(guide) = merged.as_mut().and_then(|guide| guide.guide.as_mut()) else {
        *merged = Some(proto::Guide {
            guide: Some(other.clone()),
        });
        return Ok(());
    };
    match (guide, other) {
        (Guide::Axis(axis), Guide::Axis(other)) => merge_axis(axis, other, "guide.axis", same),
        (Guide::Legend(legend), Guide::Legend(other)) => merge_legend(legend, other, same),
        (Guide::Colorbar(colorbar), Guide::Colorbar(other)) => {
            merge_colorbar(colorbar, other, same)
        }
        (Guide::None(_), Guide::None(_)) => Ok(()),
        _ => Err("set guides of two kinds".to_string()),
    }
}

/// A field that a spec sets when it is `Some`.
fn value<T: PartialEq + Clone>(merged: &mut Option<T>, other: &Option<T>, field: &str) -> Merged {
    match (merged.as_ref(), other) {
        (_, None) => Ok(()),
        (None, Some(other)) => {
            *merged = Some(other.clone());
            Ok(())
        }
        (Some(value), Some(other)) if value == other => Ok(()),
        _ => Err(differ(field)),
    }
}

/// A string field, which a spec sets when it is not empty.
fn text(merged: &mut String, other: &str, field: &str) -> Merged {
    match (merged.is_empty(), other.is_empty()) {
        (_, true) => Ok(()),
        (true, false) => {
            *merged = other.to_string();
            Ok(())
        }
        (false, false) if merged == other => Ok(()),
        _ => Err(differ(field)),
    }
}

/// An enum field, which a spec sets when it is not the zero value.
fn enumeration(merged: &mut i32, other: i32, field: &str) -> Merged {
    match (*merged, other) {
        (_, 0) => Ok(()),
        (0, other) => {
            *merged = other;
            Ok(())
        }
        (value, other) if value == other => Ok(()),
        _ => Err(differ(field)),
    }
}

/// The index of a value in `Chart.values`, which a spec sets when it is `Some`.
fn index(merged: &mut Option<u32>, other: Option<u32>, field: &str, same: &SameValue) -> Merged {
    match (*merged, other) {
        (_, None) => Ok(()),
        (None, Some(other)) => {
            *merged = Some(other);
            Ok(())
        }
        (Some(value), Some(other)) if same(value, other) => Ok(()),
        _ => Err(differ(field)),
    }
}

/// A message field, whose fields merge one by one when both specs set it.
fn message<T: Clone>(
    merged: &mut Option<T>,
    other: &Option<T>,
    merge: impl FnOnce(&mut T, &T) -> Merged,
) -> Merged {
    match (merged.as_mut(), other) {
        (_, None) => Ok(()),
        (None, Some(other)) => {
            *merged = Some(other.clone());
            Ok(())
        }
        (Some(value), Some(other)) => merge(value, other),
    }
}

fn merge_text_style(
    merged: &mut proto::TextStyle,
    other: &proto::TextStyle,
    field: &str,
) -> Merged {
    let proto::TextStyle {
        font_family,
        font_size,
        font_weight,
        color,
    } = other;
    text(
        &mut merged.font_family,
        font_family,
        &format!("{field}.font_family"),
    )?;
    value(
        &mut merged.font_size,
        font_size,
        &format!("{field}.font_size"),
    )?;
    value(
        &mut merged.font_weight,
        font_weight,
        &format!("{field}.font_weight"),
    )?;
    text(&mut merged.color, color, &format!("{field}.color"))
}

fn merge_background(
    merged: &mut proto::Background,
    other: &proto::Background,
    field: &str,
) -> Merged {
    let proto::Background {
        fill,
        stroke,
        corner_radius,
        padding,
    } = other;
    text(&mut merged.fill, fill, &format!("{field}.fill"))?;
    text(&mut merged.stroke, stroke, &format!("{field}.stroke"))?;
    value(
        &mut merged.corner_radius,
        corner_radius,
        &format!("{field}.corner_radius"),
    )?;
    value(&mut merged.padding, padding, &format!("{field}.padding"))
}

fn merge_axis(
    merged: &mut proto::Axis,
    other: &proto::Axis,
    field: &str,
    same: &SameValue,
) -> Merged {
    let proto::Axis {
        title,
        title_syntax,
        title_style,
        side,
        format,
        label,
        labels,
        label_turn,
        label_align,
        label_baseline,
        label_style,
        tick_count,
        tick_length,
        tick_color,
        domain_color,
        grid,
        grid_color,
        grid_width,
        angle,
        route,
        ticks,
    } = other;
    let at = |name: &str| format!("{field}.{name}");
    value(&mut merged.title, title, &at("title"))?;
    enumeration(&mut merged.title_syntax, *title_syntax, &at("title_syntax"))?;
    message(&mut merged.title_style, title_style, |merged, other| {
        merge_text_style(merged, other, &at("title_style"))
    })?;
    enumeration(&mut merged.side, *side, &at("side"))?;
    text(&mut merged.format, format, &at("format"))?;
    text(&mut merged.label, label, &at("label"))?;
    value(&mut merged.labels, labels, &at("labels"))?;
    value(&mut merged.label_turn, label_turn, &at("label_turn"))?;
    enumeration(&mut merged.label_align, *label_align, &at("label_align"))?;
    enumeration(
        &mut merged.label_baseline,
        *label_baseline,
        &at("label_baseline"),
    )?;
    message(&mut merged.label_style, label_style, |merged, other| {
        merge_text_style(merged, other, &at("label_style"))
    })?;
    value(&mut merged.tick_count, tick_count, &at("tick_count"))?;
    value(&mut merged.tick_length, tick_length, &at("tick_length"))?;
    text(&mut merged.tick_color, tick_color, &at("tick_color"))?;
    text(&mut merged.domain_color, domain_color, &at("domain_color"))?;
    value(&mut merged.grid, grid, &at("grid"))?;
    text(&mut merged.grid_color, grid_color, &at("grid_color"))?;
    value(&mut merged.grid_width, grid_width, &at("grid_width"))?;
    value(&mut merged.angle, angle, &at("angle"))?;
    enumeration(&mut merged.route, *route, &at("route"))?;
    // The ticks are one value: an interval's start and step, or a list.
    match (&merged.ticks, ticks) {
        (_, None) => Ok(()),
        (None, Some(ticks)) => {
            merged.ticks = Some(*ticks);
            Ok(())
        }
        (Some(axis::Ticks::TickValues(a)), Some(axis::Ticks::TickValues(b))) if same(*a, *b) => {
            Ok(())
        }
        (Some(axis::Ticks::TickInterval(a)), Some(axis::Ticks::TickInterval(b))) if a == b => {
            Ok(())
        }
        _ => Err(differ(&at("ticks"))),
    }
}

fn merge_legend(merged: &mut proto::Legend, other: &proto::Legend, same: &SameValue) -> Merged {
    let proto::Legend {
        title,
        title_syntax,
        title_style,
        format,
        label,
        label_syntax,
        label_style,
        side,
        wrap,
        label_padding,
        column_gap,
        row_gap,
        background,
        tick_count,
        values,
    } = other;
    let at = |name: &str| format!("guide.legend.{name}");
    value(&mut merged.title, title, &at("title"))?;
    enumeration(&mut merged.title_syntax, *title_syntax, &at("title_syntax"))?;
    message(&mut merged.title_style, title_style, |merged, other| {
        merge_text_style(merged, other, &at("title_style"))
    })?;
    text(&mut merged.format, format, &at("format"))?;
    text(&mut merged.label, label, &at("label"))?;
    enumeration(&mut merged.label_syntax, *label_syntax, &at("label_syntax"))?;
    message(&mut merged.label_style, label_style, |merged, other| {
        merge_text_style(merged, other, &at("label_style"))
    })?;
    enumeration(&mut merged.side, *side, &at("side"))?;
    message(&mut merged.wrap, wrap, |merged, other| {
        match (&mut merged.wrap, &other.wrap) {
            (_, None) => Ok(()),
            (None, Some(wrap)) => {
                merged.wrap = Some(*wrap);
                Ok(())
            }
            (Some(legend_wrap::Wrap::Flow(_)), Some(legend_wrap::Wrap::Flow(_))) => Ok(()),
            (Some(legend_wrap::Wrap::Grid(merged)), Some(legend_wrap::Wrap::Grid(other))) => {
                let proto::LegendGrid { columns } = other;
                value(&mut merged.columns, columns, &at("wrap.grid.columns"))
            }
            _ => Err(differ(&at("wrap"))),
        }
    })?;
    value(
        &mut merged.label_padding,
        label_padding,
        &at("label_padding"),
    )?;
    value(&mut merged.column_gap, column_gap, &at("column_gap"))?;
    value(&mut merged.row_gap, row_gap, &at("row_gap"))?;
    message(&mut merged.background, background, |merged, other| {
        merge_background(merged, other, &at("background"))
    })?;
    value(&mut merged.tick_count, tick_count, &at("tick_count"))?;
    index(&mut merged.values, *values, &at("values"), same)
}

fn merge_colorbar(
    merged: &mut proto::Colorbar,
    other: &proto::Colorbar,
    same: &SameValue,
) -> Merged {
    let proto::Colorbar {
        axis,
        thickness,
        axis_gap,
        background,
        max_length,
    } = other;
    let at = |name: &str| format!("guide.colorbar.{name}");
    message(&mut merged.axis, axis, |merged, other| {
        merge_axis(merged, other, &at("axis"), same)
    })?;
    value(&mut merged.thickness, thickness, &at("thickness"))?;
    value(&mut merged.axis_gap, axis_gap, &at("axis_gap"))?;
    message(&mut merged.background, background, |merged, other| {
        merge_background(merged, other, &at("background"))
    })?;
    value(&mut merged.max_length, max_length, &at("max_length"))
}
