//! The rules of the widgets that layers declare, each alone: its key, its selection, its column,
//! and its kind, with the domain, the step, the value, and the labels of the kind, against the
//! layer's table.

use arrow::{
    array::{Array, ArrayRef},
    compute::{cast, kernels::cmp::lt},
    datatypes::DataType,
    record_batch::RecordBatch,
};

use super::{classes, dictionary_values, fail, title, tree::Scope, Checker, Result, REQUIRED};
use crate::proto::{self, domain_fit, widget, widget_domain, SliderComparison};

/// Whether a widget's domain is an interval, for a slider, or items, for a radio group and a check
/// box list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Interval,
    Items,
}

impl Checker<'_> {
    /// Checks the widgets of a layer with `table`, whose partition columns `scope` holds.
    pub(super) fn widgets(
        &self,
        widgets: &[proto::Widget],
        table: &RecordBatch,
        scope: &Scope,
        path: &str,
    ) -> Result<()> {
        for (index, declared) in widgets.iter().enumerate() {
            let path = format!("{path}.widgets[{index}]");
            if widgets[..index]
                .iter()
                .any(|earlier| earlier.key == declared.key)
            {
                return Err(fail(
                    format!("{path}.key"),
                    format!("a layer declares the key `{}` at most once", declared.key),
                ));
            }
            self.widget(declared, table, scope, &path)?;
        }
        Ok(())
    }

    fn widget(
        &self,
        declared: &proto::Widget,
        table: &RecordBatch,
        scope: &Scope,
        path: &str,
    ) -> Result<()> {
        classes(&declared.classes, path)?;
        title(declared.title.as_ref(), &format!("{path}.title"), false)?;
        let kind = declared.kind.as_ref().expect(REQUIRED);
        let kind_path = format!("{path}.{}", kind_name(kind));
        if let widget::Kind::Mode(mode) = kind {
            return self.mode_widget(mode, &kind_path);
        }
        let column = self.target(declared.target(), table, scope, &kind_path)?;
        match kind {
            widget::Kind::Slider(slider) => {
                let path = format!("{path}.slider");
                let domain_type = self.domain(
                    slider.domain.as_ref(),
                    Family::Interval,
                    column.as_ref(),
                    &format!("{path}.domain"),
                )?;
                let equal = matches!(
                    SliderComparison::try_from(slider.comparison),
                    Ok(SliderComparison::Unspecified | SliderComparison::Equal)
                );
                if equal && slider.step.is_none() {
                    return Err(fail(
                        format!("{path}.step"),
                        "an equal slider has a step, because it writes the step's bin",
                    ));
                }
                self.step(
                    slider.step.as_ref(),
                    domain_type.as_ref(),
                    &format!("{path}.step"),
                )?;
                self.widget_labels(
                    slider.readout.as_ref(),
                    domain_type.as_ref(),
                    &format!("{path}.readout"),
                )
            }
            widget::Kind::RangeSlider(slider) => {
                let path = format!("{path}.range_slider");
                let domain_type = self.domain(
                    slider.domain.as_ref(),
                    Family::Interval,
                    column.as_ref(),
                    &format!("{path}.domain"),
                )?;
                self.step(
                    slider.step.as_ref(),
                    domain_type.as_ref(),
                    &format!("{path}.step"),
                )?;
                self.widget_labels(
                    slider.readout.as_ref(),
                    domain_type.as_ref(),
                    &format!("{path}.readout"),
                )
            }
            widget::Kind::Radio(radio) => {
                let path = format!("{path}.radio");
                let domain_type = self.domain(
                    radio.domain.as_ref(),
                    Family::Items,
                    column.as_ref(),
                    &format!("{path}.domain"),
                )?;
                self.widget_labels(
                    radio.labels.as_ref(),
                    domain_type.as_ref(),
                    &format!("{path}.labels"),
                )
            }
            widget::Kind::Checkboxes(checkboxes) => {
                let path = format!("{path}.checkboxes");
                let domain_type = self.domain(
                    checkboxes.domain.as_ref(),
                    Family::Items,
                    column.as_ref(),
                    &format!("{path}.domain"),
                )?;
                self.widget_labels(
                    checkboxes.labels.as_ref(),
                    domain_type.as_ref(),
                    &format!("{path}.labels"),
                )
            }
            widget::Kind::Checkbox(checkbox) => {
                let path = format!("{path}.checkbox.value");
                let value = self.values.scalar(checkbox.value.as_ref(), &path)?;
                same_type(value.data_type(), column.as_ref(), &path)
            }
            widget::Kind::Mode(_) => unreachable!("a mode widget returned before its target"),
        }
    }

    /// Checks that the group or the toggle of a mode widget at `path` is one of the chart's.
    fn mode_widget(&self, widget: &proto::ModeWidget, path: &str) -> Result<()> {
        let modes = &self.chart.message.modes;
        match widget.modes.as_ref().expect(REQUIRED) {
            proto::mode_widget::Modes::Group(group) => {
                if !modes.iter().any(|mode| &mode.group == group) {
                    return Err(fail(
                        format!("{path}.group"),
                        format!("the chart has no group of modes `{group}`"),
                    ));
                }
            }
            proto::mode_widget::Modes::Toggle(toggle) => {
                if !(modes.iter()).any(|mode| &mode.name == toggle && mode.group.is_empty()) {
                    return Err(fail(
                        format!("{path}.toggle"),
                        format!("the chart has no toggle `{toggle}`, a mode without a group"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Checks the target of a widget's kind at `path`, the kind's path, and returns the type of its
    /// column, or `None` for a partition column.
    fn target(
        &self,
        target: Option<&proto::WidgetTarget>,
        table: &RecordBatch,
        scope: &Scope,
        path: &str,
    ) -> Result<Option<DataType>> {
        let path = format!("{path}.target");
        let target = target.expect(REQUIRED);
        self.selection(&target.selection, &format!("{path}.selection"))?;
        match table.column_by_name(&target.column) {
            Some(column) => Ok(Some(dictionary_values(column.data_type()).clone())),
            None if scope.partition.contains(&target.column) => Ok(None),
            None => Err(fail(
                format!("{path}.column"),
                format!(
                    "the layer's table and its partition have no column `{}`",
                    target.column
                ),
            )),
        }
    }

    /// Checks a widget's domain of `family` over a column of `column`'s type, or of an unknown type
    /// for a partition column, and returns the domain's type, when it is known.
    fn domain(
        &self,
        domain: Option<&proto::WidgetDomain>,
        family: Family,
        column: Option<&DataType>,
        path: &str,
    ) -> Result<Option<DataType>> {
        let domain = (domain.and_then(|domain| domain.domain.as_ref())).expect(REQUIRED);
        let (values, fit) = match domain {
            widget_domain::Domain::Explicit(values) => {
                let path = format!("{path}.explicit");
                let values = self.values.list(values, &path)?;
                match family {
                    Family::Interval => ascending_ends(&values, &path)?,
                    Family::Items if values.is_empty() => {
                        return Err(fail(&path, "a widget's explicit items are one or more"))
                    }
                    Family::Items => {}
                }
                (Some(values), None)
            }
            widget_domain::Domain::Summary(summary) => {
                let values = self
                    .values
                    .list(summary.values.as_ref(), &format!("{path}.summary.values"))?;
                (
                    Some(values),
                    summary.fit.as_ref().map(|fit| (fit, "summary")),
                )
            }
            widget_domain::Domain::Data(data) => (None, data.fit.as_ref().map(|fit| (fit, "data"))),
        };
        if let Some((fit, kind)) = fit {
            let path = format!("{path}.{kind}.fit");
            match (family, fit.fit.as_ref()) {
                (_, None) | (Family::Interval, Some(domain_fit::Fit::Interval(_))) => {}
                (Family::Items, Some(domain_fit::Fit::Categories(_))) => {}
                (Family::Interval, Some(_)) => {
                    return Err(fail(path, "a slider's domain fits as an interval"))
                }
                (Family::Items, Some(_)) => {
                    return Err(fail(path, "the items of a widget fit as categories"))
                }
            }
        }
        let domain_type = match &values {
            Some(values) => {
                let data_type = dictionary_values(values.data_type()).clone();
                same_type(&data_type, column, path)?;
                Some(data_type)
            }
            None => column.cloned(),
        };
        if let (Family::Interval, Some(data_type)) = (family, &domain_type) {
            if !(data_type.is_numeric() || is_temporal(data_type)) {
                return Err(fail(
                    path,
                    format!("a slider runs over numbers, dates, or timestamps, not {data_type}"),
                ));
            }
        }
        Ok(domain_type)
    }

    /// Checks a slider's step: a number for a domain of numbers, or a duration for a domain of dates
    /// or timestamps, more than 0.
    fn step(
        &self,
        step: Option<&proto::ValueRef>,
        domain: Option<&DataType>,
        path: &str,
    ) -> Result<()> {
        let Some(step) = step else {
            return Ok(());
        };
        let step = self.values.scalar(step, path)?;
        let data_type = step.data_type();
        let fits = match domain {
            Some(domain) if is_temporal(domain) => matches!(data_type, DataType::Duration(_)),
            Some(_) => data_type.is_numeric(),
            None => data_type.is_numeric() || matches!(data_type, DataType::Duration(_)),
        };
        if !fits {
            return Err(fail(
                path,
                "a step is a number for a domain of numbers, and a duration for a domain of dates \
                 or timestamps",
            ));
        }
        let as_number = match data_type {
            DataType::Duration(_) => {
                cast(step, &DataType::Int64).and_then(|step| cast(&step, &DataType::Float64))
            }
            _ => cast(step, &DataType::Float64),
        }
        .map_err(|error| fail(path, error.to_string()))?;
        let positive = (as_number.as_any())
            .downcast_ref::<arrow::array::Float64Array>()
            .is_some_and(|number| number.is_valid(0) && number.value(0) > 0.0);
        if !positive {
            return Err(fail(path, "a step is more than 0"));
        }
        Ok(())
    }

    /// Checks the pattern of labels or of a readout for values of the domain's type.
    fn widget_labels(
        &self,
        labels: Option<&proto::Labels>,
        domain: Option<&DataType>,
        path: &str,
    ) -> Result<()> {
        match (labels, domain) {
            (Some(labels), Some(domain)) => {
                self.pattern(&labels.format, domain, &format!("{path}.format"))
            }
            _ => Ok(()),
        }
    }
}

/// The name of the field of a widget's kind.
fn kind_name(kind: &widget::Kind) -> &'static str {
    match kind {
        widget::Kind::Slider(_) => "slider",
        widget::Kind::RangeSlider(_) => "range_slider",
        widget::Kind::Radio(_) => "radio",
        widget::Kind::Checkboxes(_) => "checkboxes",
        widget::Kind::Checkbox(_) => "checkbox",
        widget::Kind::Mode(_) => "mode",
    }
}

fn is_temporal(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _)
    )
}

/// Fails for values of `data_type` in a widget over a column of another type.
fn same_type(data_type: &DataType, column: Option<&DataType>, path: &str) -> Result<()> {
    match column {
        Some(column) if dictionary_values(data_type) != column => Err(fail(
            path,
            format!("the widget's values are {data_type}, and its column holds {column}"),
        )),
        _ => Ok(()),
    }
}

/// Checks the explicit domain of a slider: two ends, not null, in ascending order.
fn ascending_ends(values: &ArrayRef, path: &str) -> Result<()> {
    if values.len() != 2 || values.null_count() > 0 {
        return Err(fail(
            path,
            "a slider's explicit domain holds two ends that are not null",
        ));
    }
    let less = lt(&values.slice(1, 1), &values.slice(0, 1))
        .map_err(|error| fail(path, error.to_string()))?;
    if less.true_count() > 0 {
        return Err(fail(
            path,
            "a slider's explicit ends are in ascending order",
        ));
    }
    Ok(())
}
