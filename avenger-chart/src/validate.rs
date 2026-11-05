//! The rules of the format that a chart must follow, which a reader checks before it reads one.
//! Each error names the path of the field that breaks a rule.

use crate::{
    codec::FORMAT_VERSION,
    walk::{VisitTables, VisitValues},
    Chart, ChartError,
};

type Result<T> = std::result::Result<T, ChartError>;

/// Checks `chart` against the rules of the format.
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
        return Err(ChartError::invalid(
            "tables",
            "a chart holds its tables and its values decoded, so its message holds neither",
        ));
    }
    let root = (message.root.as_ref()).ok_or_else(|| ChartError::invalid("root", "is not set"))?;
    references(chart, root)?;
    Ok(())
}

/// Fails for a `TableRef` or a `ValueRef` past the end of its list.
fn references(chart: &Chart, root: &crate::proto::Node) -> Result<()> {
    let mut failure = None;
    root.visit_tables(&mut |table| {
        if table.index as usize >= chart.tables.len() && failure.is_none() {
            failure = Some(ChartError::invalid(
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
            failure = Some(ChartError::invalid(
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
