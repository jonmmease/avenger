//! The state of a live chart as text, for a host that shows it apart from the chart, as a debugger's
//! panes do: each selection's clauses, each widget's value, each scale's domain as the chart draws
//! it, and the modes that are on, and a line that names what a gesture changed.

use arrow::array::{Array, ArrayRef};
use arrow::util::display::{ArrayFormatter, FormatOptions};
use avenger_chart::proto::Channel;
use avenger_chart::resolve::{Id, Identity};
use avenger_chart::FitKind;
use avenger_chart_app::{ChartAppState, ChartChange};
use avenger_chart_eval::{ScaleIdentity, WidgetValue};
use avenger_selection::{Clause, Condition};

/// The state of a live chart, as text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartState {
    /// Each selection by its name, with a line for each of its clauses, or none when it is clear.
    pub selections: Vec<(String, Vec<String>)>,
    /// Each widget by its key, with its value.
    pub widgets: Vec<(String, String)>,
    /// Each scale of the chart.
    pub scales: Vec<ScaleState>,
    /// The modes that are on.
    pub modes: Vec<String>,
}

/// A scale of a live chart, as text.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleState {
    /// The scale's identity, by which a change's line names it.
    pub identity: ScaleIdentity,
    /// Its name, its channel and its plots, as `x of cars`, `color of plot 1 (Origin = Europe)`,
    /// or `x of plots 1–3`.
    pub name: String,
    /// Its domain as the chart draws it: an interval as `50.0 to 200.0`, the visible one for a
    /// panned or zoomed position scale, or categories as `Europe, Japan, USA`.
    pub domain: String,
}

/// The state of the chart app `state`.
pub(crate) fn chart_state(state: &ChartAppState) -> ChartState {
    let evaluator = state.evaluator();
    let plots: Vec<String> = (state.plots().iter().enumerate())
        .map(|(index, plot)| plot_name(index, &plot.identity))
        .collect();
    let selections = (evaluator.selections().iter())
        .map(|(name, selection)| (name.clone(), selection.clauses.iter().map(clause).collect()))
        .collect();
    let widgets = (state.widgets().iter())
        .map(|widget| {
            (
                widget.key.clone(),
                widget_value(&evaluator.widget_value(widget)),
            )
        })
        .collect();
    let scales = (evaluator.scales().into_iter())
        .map(|scale| ScaleState {
            name: scale_name(&scale.identity, &scale.plots, &plots),
            domain: match scale.kind {
                FitKind::Interval => values(scale.scale.domain(), " to "),
                FitKind::Categories | FitKind::Sample | FitKind::Explicit => {
                    categories(scale.scale.domain())
                }
            },
            identity: scale.identity,
        })
        .collect();
    ChartState {
        selections,
        widgets,
        scales,
        modes: evaluator.modes().into_iter().map(str::to_string).collect(),
    }
}

/// A line that names what `change` changed, as `brush: Horsepower 80 to 140`, or none. Its scales
/// name their plots as the chart's state `state` does.
pub fn describe_change(change: &ChartChange, state: &ChartState) -> Option<String> {
    let mut parts = Vec::new();
    for (name, clauses) in &change.selections {
        let value = match clauses.is_empty() {
            true => "clear".to_string(),
            false => clauses.iter().map(clause).collect::<Vec<_>>().join("; "),
        };
        parts.push(format!("{name}: {value}"));
    }
    for (identity, domain) in &change.domains {
        let name = (state.scales.iter())
            .find(|scale| scale.identity == *identity)
            .map_or_else(
                || scale_name(identity, &[], &[]),
                |scale| scale.name.clone(),
            );
        parts.push(format!("{name}: {}", values(domain, " to ")));
    }
    if let Some(modes) = &change.modes {
        let modes = match modes.is_empty() {
            true => "none".to_string(),
            false => modes.join(", "),
        };
        parts.push(format!("modes: {modes}"));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// A clause as text: its conditions, as `Horsepower 80 to 140` or `Origin in USA, Japan`.
fn clause(clause: &Clause) -> String {
    let conditions: Vec<String> = (clause.conditions.iter())
        .map(|condition| match condition {
            Condition::Range { column, ends, .. } => format!("{column} {}", values(ends, " to ")),
            Condition::Set {
                columns,
                values: sets,
                complement,
                ..
            } => {
                let not = if *complement { "not " } else { "" };
                let sets: Vec<String> = (columns.iter().zip(sets))
                    .map(|(column, set)| format!("{column} {not}in {}", values(set, ", ")))
                    .collect();
                sets.join(" and ")
            }
            Condition::Polygon {
                x_column, y_column, ..
            } => format!("a lasso of {x_column} and {y_column}"),
            #[allow(unreachable_patterns)]
            other => format!("{other:?}"),
        })
        .collect();
    conditions.join(" and ")
}

/// The name of the plot at `index` among a chart's plots, whose identity is `identity`: its id,
/// or its place, as `plot 2`, with the values of its facet, as `plot 2 (Origin = Europe)`.
pub fn plot_name(index: usize, identity: &Identity) -> String {
    let name = match identity.given_id() {
        Some(id) => id.to_string(),
        None => format!("plot {}", index + 1),
    };
    if identity.partition.is_empty() {
        return name;
    }
    let values: Vec<String> = (identity.partition.iter())
        .map(|(key, value)| format!("{key} = {}", texts(value, 1).join("")))
        .collect();
    format!("{name} ({})", values.join(", "))
}

/// The name of `channel` as a chart's text names it, as `x` or `stroke_width`.
pub fn channel_name(channel: Channel) -> String {
    (channel.as_str_name().trim_start_matches("CHANNEL_")).to_lowercase()
}

/// The name of a scale: its channel and its plots, `plots` by their indices in `names`, the
/// chart's plots' names, as `x of cars`, or `x of plots 1–3` for a scale of several plots, or its
/// family. A scale whose plots are unknown takes the id or the path of its plot.
fn scale_name(identity: &ScaleIdentity, plots: &[usize], names: &[String]) -> String {
    match identity {
        ScaleIdentity::Family { family, channel: c } => format!("{} of {family}", channel_name(*c)),
        ScaleIdentity::Plot {
            plot,
            channel: c,
            index,
        } => {
            let of = match plots {
                [one] if *one < names.len() => names[*one].clone(),
                [first, .., last] if *last - *first + 1 == plots.len() => {
                    format!("plots {}–{}", first + 1, last + 1)
                }
                [_, _, ..] => {
                    let places: Vec<String> = plots.iter().map(|p| (p + 1).to_string()).collect();
                    format!("plots {}", places.join(", "))
                }
                _ => match &plot.id {
                    Id::Given(id) => id.clone(),
                    Id::Path(path) => path.clone(),
                },
            };
            let second = if *index > 0 { " (second)" } else { "" };
            format!("{} of {of}{second}", channel_name(*c))
        }
    }
}

/// The most values of a domain of categories that its text lists.
const CATEGORIES: usize = 20;

/// The values of `array` as text, joined by `separator`. A negative zero reads `0`.
fn values(array: &ArrayRef, separator: &str) -> String {
    texts(array, array.len()).join(separator)
}

/// The values of a domain of categories, as `Europe, Japan, USA`, the first [`CATEGORIES`] of a
/// longer one with the count of the rest.
fn categories(array: &ArrayRef) -> String {
    let mut text = texts(array, CATEGORIES).join(", ");
    if array.len() > CATEGORIES {
        text.push_str(&format!(", and {} more", array.len() - CATEGORIES));
    }
    text
}

/// The first `count` values of `array` as text.
fn texts(array: &ArrayRef, count: usize) -> Vec<String> {
    let options = FormatOptions::default().with_null("NULL");
    let Ok(formatter) = ArrayFormatter::try_new(array.as_ref(), &options) else {
        return vec![format!("{array:?}")];
    };
    (0..array.len().min(count))
        .map(|index| {
            let text = formatter.value(index).to_string();
            match text.strip_prefix('-') {
                Some(rest) if text.parse::<f64>() == Ok(0.0) => rest.to_string(),
                _ => text,
            }
        })
        .collect()
}

/// A widget's value as text.
fn widget_value(value: &WidgetValue) -> String {
    match value {
        WidgetValue::Number(number) => number.to_string(),
        WidgetValue::Range([low, high]) => format!("{low} to {high}"),
        WidgetValue::Item(Some(item)) => format!("item {}", item + 1),
        WidgetValue::Item(None) => "none".to_string(),
        other => format!("{other:?}"),
    }
}
