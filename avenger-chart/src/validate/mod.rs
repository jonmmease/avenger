//! The rules of the format that a chart must follow, which a reader checks before it reads one.
//! Each error names the path of the field that breaks a rule, such as
//! `root.plot.cartesian.layers[0].symbol.x.scale`.

mod encoding;
mod selection;
mod shared;
mod tree;
mod widget;

pub use encoding::FitKind;
pub(crate) use encoding::{is_position, scaled, scales_numbers, scales_strings, takes_scale};

use std::collections::HashSet;

use arrow::{
    array::{Array, ArrayRef, AsArray},
    datatypes::DataType,
    row::{RowConverter, SortField},
};

use crate::{
    codec::FORMAT_VERSION,
    proto,
    walk::{enum_error, VisitEnums, VisitRequired, VisitTables, VisitValues},
    Chart, ChartError,
};

type Result<T> = std::result::Result<T, ChartError>;

/// Why a required field is present in the hand-written checks, which run after the generated
/// checks of the required fields and oneofs.
const REQUIRED: &str = "the generated checks found the required fields";

/// An error at `path`.
fn fail(path: impl Into<String>, reason: impl Into<String>) -> ChartError {
    ChartError::invalid(path, reason)
}

/// Checks `chart` against the rules of the format that each of its messages decides alone.
pub(crate) fn check(chart: &Chart) -> Result<()> {
    let message = &chart.message;
    match message.version {
        FORMAT_VERSION => {}
        0 => {
            return Err(ChartError::NotAChart(
                "its format version is 0, which no chart has".to_string(),
            ))
        }
        version => return Err(ChartError::UnsupportedVersion(version)),
    }
    if !message.tables.is_empty() || message.values.is_some() {
        return Err(fail(
            "tables",
            "a chart holds its tables and its values decoded, so its message holds neither",
        ));
    }
    if let Some(index) = chart.values.iter().position(|value| value.len() != 1) {
        return Err(fail(
            "values",
            format!(
                "value {index} has {} elements, and each value is one element",
                chart.values[index].len()
            ),
        ));
    }
    required(message)?;
    let root = (message.root.as_ref()).expect("`required` checked the root");
    references(chart, root)?;
    enums(message)?;
    size(message.width, "width")?;
    size(message.height, "height")?;
    if let Some(padding) = &message.padding {
        for (side, value) in [
            ("top", padding.top),
            ("right", padding.right),
            ("bottom", padding.bottom),
            ("left", padding.left),
        ] {
            non_negative(value.map(f64::from), &format!("padding.{side}"))?;
        }
    }
    let selections = selection::selections(chart);
    let checker = Checker {
        chart,
        values: Values(&chart.values),
        selections,
    };
    checker.node(root, "root", &tree::Scope::default())?;
    checker.initial_clauses()?;
    Ok(())
}

/// Checks the rules that span the messages of `chart`, whose messages are each valid.
pub(crate) fn check_shared(chart: &Chart) -> Result<()> {
    let checker = Checker {
        chart,
        values: Values(&chart.values),
        selections: selection::selections(chart),
    };
    checker.shared()
}

/// What the checks of a chart's parts read: the chart, its values, and the names of its
/// selections.
struct Checker<'a> {
    chart: &'a Chart,
    values: Values<'a>,
    selections: HashSet<&'a str>,
}

impl Checker<'_> {
    /// Fails for a name of a selection that the chart does not define.
    fn selection(&self, name: &str, path: &str) -> Result<()> {
        if !self.selections.contains(name) {
            return Err(fail(path, format!("the chart has no selection `{name}`")));
        }
        Ok(())
    }

    /// Checks the modes of a tool at `path`: each names a mode of the chart, once.
    fn tool_modes(&self, modes: &[String], path: &str) -> Result<()> {
        for (index, name) in modes.iter().enumerate() {
            let path = format!("{path}[{index}]");
            if !(self.chart.message.modes.iter()).any(|mode| &mode.name == name) {
                return Err(fail(path, format!("the chart has no mode `{name}`")));
            }
            if modes[..index].contains(name) {
                return Err(fail(path, format!("the tool names the mode `{name}` once")));
            }
        }
        Ok(())
    }

    /// Fails for a pattern of numbers or of dates and datetimes, for values of `data_type`, in a
    /// chart that does not name the syntax of such patterns.
    fn pattern(&self, pattern: &str, data_type: &DataType, path: &str) -> Result<()> {
        if pattern.is_empty() {
            return Ok(());
        }
        let message = &self.chart.message;
        let data_type = dictionary_values(data_type);
        if data_type.is_numeric() && message.number_format_syntax.is_empty() {
            return Err(fail(
                path,
                "a chart with a pattern of numbers names their syntax in `number_format_syntax`",
            ));
        }
        if data_type.is_temporal() && message.datetime_format_syntax.is_empty() {
            return Err(fail(
                path,
                "a chart with a pattern of dates or datetimes names their syntax in \
                 `datetime_format_syntax`",
            ));
        }
        Ok(())
    }
}

/// Fails for a `TableRef` or a `ValueRef` past the end of its list.
fn references(chart: &Chart, root: &proto::Node) -> Result<()> {
    let mut failure = None;
    root.visit_tables(&mut |table| {
        if table.index as usize >= chart.tables.len() && failure.is_none() {
            failure = Some(fail(
                "tables",
                format!(
                    "a layer refers to table {}, and the chart has {} tables",
                    table.index,
                    chart.tables.len()
                ),
            ));
        }
    });
    chart.message.visit_values(&mut |value| {
        if value.index as usize >= chart.values.len() && failure.is_none() {
            failure = Some(fail(
                "values",
                format!(
                    "a field refers to value {}, and the chart has {} values",
                    value.index,
                    chart.values.len()
                ),
            ));
        }
    });
    failure.map_or(Ok(()), Err)
}

/// Fails for the first field or oneof that the options of `annotation.proto` require and that is
/// absent, so the other checks read every required field as present.
fn required(message: &proto::Chart) -> Result<()> {
    let mut failure = None;
    message.visit_required("", &mut |path, reason| {
        failure.get_or_insert_with(|| fail(path, reason));
    });
    failure.map_or(Ok(()), Err)
}

/// Fails for an enum field whose value its enum does not declare.
fn enums(message: &proto::Chart) -> Result<()> {
    let mut failure = None;
    if let Some(root) = &message.root {
        root.visit_enums("root", &mut |path, name, value| {
            if failure.is_none() {
                failure =
                    enum_error(name, value).map(|reason| fail(path, format!("{value} {reason}")));
            }
        });
    }
    for (index, selection) in message.selections.iter().enumerate() {
        selection.visit_enums(&format!("selections[{index}]"), &mut |path, name, value| {
            if failure.is_none() {
                failure =
                    enum_error(name, value).map(|reason| fail(path, format!("{value} {reason}")));
            }
        });
    }
    failure.map_or(Ok(()), Err)
}

/// The values of a chart, by `ValueRef`.
#[derive(Clone, Copy)]
struct Values<'a>(&'a [ArrayRef]);

impl<'a> Values<'a> {
    /// The value that `value` refers to, an array of one element. A field that must refer to a
    /// value and refers to none fails.
    fn scalar<'r>(
        self,
        value: impl Into<Option<&'r proto::ValueRef>>,
        path: &str,
    ) -> Result<&'a ArrayRef> {
        let index = (value.into())
            .ok_or_else(|| fail(path, "the field always refers to a value"))?
            .index;
        // The references are in range, which `references` checked.
        Ok(&self.0[index as usize])
    }

    /// The elements of the list that `value` refers to.
    fn list<'r>(
        self,
        value: impl Into<Option<&'r proto::ValueRef>>,
        path: &str,
    ) -> Result<ArrayRef> {
        let value = self.scalar(value, path)?;
        match value.as_list_opt::<i32>() {
            Some(list) if list.is_valid(0) => Ok(list.value(0)),
            _ => Err(fail(
                path,
                format!(
                    "the field holds a list that is not null, not a value of type {}",
                    value.data_type()
                ),
            )),
        }
    }

    /// The key of the value that `value` refers to: its type and the row of its value, which are
    /// equal exactly when two values are equal.
    fn key(self, value: Option<&proto::ValueRef>, path: &str) -> Result<Vec<u8>> {
        value_key(self.scalar(value, path)?).map_err(|reason| fail(path, reason))
    }
}

/// The key of `value`, an array of one element: its type and the row of its value, which are
/// equal exactly when two values are equal.
pub(crate) fn value_key(value: &ArrayRef) -> std::result::Result<Vec<u8>, String> {
    let converter = RowConverter::new(vec![SortField::new(value.data_type().clone())])
        .map_err(|error| error.to_string())?;
    let rows = (converter.convert_columns(std::slice::from_ref(value)))
        .map_err(|error| error.to_string())?;
    let mut key = format!("{}", value.data_type()).into_bytes();
    key.push(0);
    key.extend_from_slice(rows.row(0).as_ref());
    Ok(key)
}

/// Fails for entries of a keyed list that share a key and differ: equal entries with one key count
/// as one, and different ones are a conflict.
fn keyed<T, K: PartialEq + std::fmt::Debug>(
    items: &[T],
    key: impl Fn(&T) -> K,
    equal: impl Fn(&T, &T) -> bool,
    path: &str,
    what: &str,
) -> Result<()> {
    for (index, item) in items.iter().enumerate() {
        for earlier in &items[..index] {
            if key(earlier) == key(item) && !equal(earlier, item) {
                return Err(fail(
                    path,
                    format!(
                        "two {what} with the key {:?} differ, which is a conflict",
                        key(item)
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Fails for a size that is set and is not more than 0 and finite.
fn size(value: Option<f32>, path: &str) -> Result<()> {
    if value.is_some_and(|value| !(value.is_finite() && value > 0.0)) {
        return Err(fail(path, "a size is more than 0 and finite"));
    }
    Ok(())
}

/// Fails for a value that is set and is not at least 0 and finite.
fn non_negative(value: Option<f64>, path: &str) -> Result<()> {
    if value.is_some_and(|value| !(value.is_finite() && value >= 0.0)) {
        return Err(fail(path, "the value is at least 0 and finite"));
    }
    Ok(())
}

/// Fails for a value that is set and is not more than 0 and finite.
fn positive(value: Option<f64>, path: &str) -> Result<()> {
    if value.is_some_and(|value| !(value.is_finite() && value > 0.0)) {
        return Err(fail(path, "the value is more than 0 and finite"));
    }
    Ok(())
}

/// Fails for a value that is set and is not from `low` to `high`.
fn within(value: Option<f64>, low: f64, high: f64, path: &str) -> Result<()> {
    if value.is_some_and(|value| !(low..=high).contains(&value)) {
        return Err(fail(path, format!("the value is from {low} to {high}")));
    }
    Ok(())
}

/// Fails for an empty name.
fn named(name: &str, path: &str, what: &str) -> Result<()> {
    if name.is_empty() {
        return Err(fail(path, format!("{what} is not empty")));
    }
    Ok(())
}

pub(crate) fn dictionary_values(data_type: &DataType) -> &DataType {
    match data_type {
        DataType::Dictionary(_, values) => values,
        other => other,
    }
}

fn is_string(data_type: &DataType) -> bool {
    matches!(
        dictionary_values(data_type),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    )
}

/// Checks a title at `path`: the title of a part other than a node, such as an axis, has none of
/// the fields that place the title of a node and give its subtitle.
fn title(title: Option<&proto::Title>, path: &str, node: bool) -> Result<()> {
    let Some(title) = title else { return Ok(()) };
    if !node {
        let fields = [
            ("side", title.side != 0),
            ("align", title.align != 0),
            ("subtitle", !title.subtitle.is_empty()),
            ("subtitle_style", title.subtitle_style.is_some()),
        ];
        if let Some((field, _)) = fields.iter().find(|(_, set)| *set) {
            return Err(fail(
                format!("{path}.{field}"),
                "only the title of a plot, a concat, or a wrap has a side, an alignment, or a subtitle",
            ));
        }
    }
    Ok(())
}

/// Checks the class names of a node, which stylesheets select: each is not empty and holds no
/// whitespace.
fn classes(classes: &[String], path: &str) -> Result<()> {
    for (index, class) in classes.iter().enumerate() {
        if class.is_empty() || class.chars().any(char::is_whitespace) {
            return Err(fail(
                format!("{path}.classes[{index}]"),
                "a class is not empty and holds no whitespace",
            ));
        }
    }
    Ok(())
}
