//! The rules of marks and their encodings: the channels that a mark needs, the values and the
//! scales of encodings, the domains, fits, ranges, and guides of scales, and the patterns that
//! format their labels.

use std::collections::HashMap;

use arrow::{
    array::{Array, ArrayRef},
    compute::kernels::cmp::lt,
    datatypes::{DataType, Fields},
    record_batch::RecordBatch,
    row::{RowConverter, SortField},
};

use super::{
    classes, dictionary_values, fail, is_string, keyed, named, non_negative, positive, within,
    Checker, Result,
};
use crate::{
    layer::{CoordKind, LayerRef, MarkMessage, MarkType},
    proto::{
        self, category_fit, conditional_value, domain, domain_fit, encoding, guide, guide_ticks,
        legend, navigation_limits, polar_layer, range, scale_spec, selection_domain, Channel,
    },
    resolve::scale_channel,
};

/// The kind of fit of a scale type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitKind {
    /// Linear, log, pow, symlog, time, and quantize scales.
    Interval,
    /// Band, point, nested band, and ordinal scales.
    Categories,
    /// Quantile scales, whose summaries are samples and which have no fit.
    Sample,
    /// Threshold scales, which take only an explicit domain.
    Explicit,
}

impl FitKind {
    /// The kind of fit of the scale type `scale_type`, or `None` for a custom scale, whose kind
    /// its renderer knows.
    pub fn of(scale_type: &scale_spec::Type) -> Option<Self> {
        use scale_spec::Type;
        Some(match scale_type {
            Type::Linear(_)
            | Type::Log(_)
            | Type::Pow(_)
            | Type::Symlog(_)
            | Type::Time(_)
            | Type::Quantize(_) => Self::Interval,
            Type::Band(_) | Type::Point(_) | Type::NestedBand(_) | Type::Ordinal(_) => {
                Self::Categories
            }
            Type::Quantile(_) => Self::Sample,
            Type::Threshold(_) => Self::Explicit,
            Type::Custom(_) => return None,
        })
    }
}

/// The values that a scale type's domain holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DomainValues {
    Numbers,
    Times,
    Categories,
    Paths,
    Any,
}

fn domain_values(scale_type: &scale_spec::Type) -> DomainValues {
    use scale_spec::Type;
    match scale_type {
        Type::Linear(_)
        | Type::Log(_)
        | Type::Pow(_)
        | Type::Symlog(_)
        | Type::Quantize(_)
        | Type::Quantile(_)
        | Type::Threshold(_) => DomainValues::Numbers,
        Type::Time(_) => DomainValues::Times,
        Type::Band(_) | Type::Point(_) | Type::Ordinal(_) => DomainValues::Categories,
        Type::NestedBand(_) => DomainValues::Paths,
        Type::Custom(_) => DomainValues::Any,
    }
}

fn takes_values(values: DomainValues, data_type: &DataType) -> bool {
    let data_type = dictionary_values(data_type);
    match values {
        DomainValues::Numbers => data_type.is_numeric(),
        DomainValues::Times => data_type.is_temporal(),
        DomainValues::Categories | DomainValues::Any => true,
        DomainValues::Paths => matches!(data_type, DataType::Struct(_)),
    }
}

/// The name of a scale type in errors.
fn type_name(scale_type: &scale_spec::Type) -> &'static str {
    use scale_spec::Type;
    match scale_type {
        Type::Linear(_) => "linear",
        Type::Log(_) => "log",
        Type::Pow(_) => "pow",
        Type::Symlog(_) => "symlog",
        Type::Time(_) => "time",
        Type::Band(_) => "band",
        Type::Point(_) => "point",
        Type::NestedBand(_) => "nested band",
        Type::Ordinal(_) => "ordinal",
        Type::Quantile(_) => "quantile",
        Type::Quantize(_) => "quantize",
        Type::Threshold(_) => "threshold",
        Type::Custom(_) => "custom",
    }
}

/// Whether the legends and colorbars of a scale type show ticks, unlike the bins of quantize,
/// quantile, and threshold scales.
fn ticked(scale_type: &scale_spec::Type) -> bool {
    use scale_spec::Type;
    matches!(
        scale_type,
        Type::Linear(_) | Type::Log(_) | Type::Pow(_) | Type::Symlog(_) | Type::Time(_)
    )
}

impl Checker<'_> {
    /// Checks a layer's mark against its table.
    pub(super) fn layer(&self, layer: LayerRef, table: &RecordBatch, path: &str) -> Result<()> {
        let mark = layer
            .mark()
            .ok_or_else(|| fail(format!("{path}.mark"), "the mark of a layer is required"))?;
        let mark_path = format!("{path}.mark.{}", mark.name());
        for (channel, encoding) in mark.encodings() {
            let path = format!("{mark_path}.{}", channel.field_name());
            let first = scale_channel(channel);
            if first != channel && mark.encoding(first).is_none() {
                return Err(fail(
                    &path,
                    format!(
                        "`{}` is the second channel of `{}`, which the mark does not encode",
                        channel.field_name(),
                        first.field_name()
                    ),
                ));
            }
            self.encoding(encoding, channel, layer.kind(), table, &path)?;
        }
        let required: &[Channel] = match mark.mark_type() {
            MarkType::Text => &[Channel::Text],
            MarkType::Image => &[Channel::Image],
            MarkType::Path => &[Channel::Path],
            MarkType::Shape => &[Channel::Geometry],
            MarkType::Rule if layer.kind() == CoordKind::Geo => {
                &[Channel::Longitude, Channel::Latitude]
            }
            _ => &[],
        };
        if let Some(missing) = required
            .iter()
            .find(|channel| mark.encoding(**channel).is_none())
        {
            return Err(fail(
                &mark_path,
                format!("a `{}` mark needs `{}`", mark.name(), missing.field_name()),
            ));
        }
        if mark.mark_type() == MarkType::Rect {
            rect_spans(mark, layer.kind(), &mark_path)?;
        }
        if let LayerRef::Polar(proto::PolarLayer {
            mark: Some(polar_layer::Mark::Bar(bar)),
            ..
        }) = layer
        {
            polar_bar_spans(mark, bar, &mark_path)?;
        }
        if let Some(text) = text_props(layer) {
            text.check(mark.encoding(Channel::Width).is_some(), &mark_path)?;
            if let Some(encoding) = mark.encoding(Channel::Text) {
                let data_type = self.value_type(encoding, table, &mark_path)?;
                self.pattern(text.format, &data_type, &format!("{mark_path}.format"))?;
            }
        }
        one_value_per_series(mark, table, &mark_path)
    }

    /// The type of the values of `encoding`: of its column, or of its constant.
    fn value_type(
        &self,
        encoding: &proto::Encoding,
        table: &RecordBatch,
        path: &str,
    ) -> Result<DataType> {
        Ok(match encoding.value.as_ref() {
            Some(encoding::Value::Column(name)) => column(table, name, path)?.data_type().clone(),
            Some(encoding::Value::Constant(value) | encoding::Value::ScaledConstant(value)) => {
                self.values.scalar(value, path)?.data_type().clone()
            }
            None => return Err(fail(path, "the value of an encoding is required")),
        })
    }

    fn encoding(
        &self,
        encoding: &proto::Encoding,
        channel: Channel,
        kind: CoordKind,
        table: &RecordBatch,
        path: &str,
    ) -> Result<()> {
        let value = (encoding.value.as_ref())
            .ok_or_else(|| fail(path, "the value of an encoding is required"))?;
        let value_type = self.value_type(encoding, table, path)?;
        if channel == Channel::Geometry {
            let geoarrow = match value {
                encoding::Value::Column(name) => (table.schema().field_with_name(name).ok())
                    .and_then(|field| field.extension_type_name().map(str::to_string))
                    .is_some_and(|extension| extension.starts_with("geoarrow.")),
                _ => false,
            };
            if !geoarrow {
                return Err(fail(
                    path,
                    "a `geometry` encoding is a column of GeoArrow geometries, whose field names a \
                     `geoarrow` extension type",
                ));
            }
        }
        if matches!(value, encoding::Value::Constant(_))
            && matches!(channel, Channel::X | Channel::Y | Channel::X2 | Channel::Y2)
        {
            return Err(fail(
                path,
                "a Cartesian position is a column or a scaled constant, which go through the scale \
                 of `x` or `y`",
            ));
        }
        let scaled = matches!(value, encoding::Value::ScaledConstant(_))
            || (encoding.conditional.iter()).any(|conditional| {
                matches!(
                    conditional.value,
                    Some(conditional_value::Value::ScaledConstant(_))
                )
            });
        let constants = matches!(value, encoding::Value::Constant(_))
            && (encoding.conditional.iter()).all(|conditional| {
                matches!(
                    conditional.value,
                    Some(conditional_value::Value::Constant(_))
                )
            });
        match (&encoding.scale, takes_scale(channel)) {
            (Some(_), false) => {
                return Err(fail(
                    path,
                    format!("`{}` never has a scale", channel.field_name()),
                ))
            }
            (Some(_), true) if constants => {
                return Err(fail(
                    path,
                    "an encoding whose values are all constants has no scale",
                ))
            }
            (None, true) if scaled => {
                return Err(fail(
                    path,
                    "a scaled constant goes through the channel's scale",
                ))
            }
            (None, true) if matches!(channel, Channel::X | Channel::Y) => {
                return Err(fail(
                    path,
                    "`x` and `y` always have a scale, because the axes and the fit of the plot \
                     need one",
                ))
            }
            _ => {}
        }
        match &encoding.scale {
            Some(scale) => {
                self.scale(scale, channel, kind, &value_type, &format!("{path}.scale"))?
            }
            None => raw_values(channel, &value_type, path)?,
        }
        if !encoding.conditional.is_empty() && (is_position(channel) || channel == Channel::Detail)
        {
            return Err(fail(
                path,
                "the position channels and `detail` have no conditional values",
            ));
        }
        for (index, conditional) in encoding.conditional.iter().enumerate() {
            let path = format!("{path}.conditional[{index}]");
            let predicate = (conditional.predicate.as_ref())
                .ok_or_else(|| fail(&path, "the predicate of a conditional value is required"))?;
            self.predicate(predicate, &format!("{path}.predicate"))?;
            match (conditional.value.as_ref())
                .ok_or_else(|| fail(&path, "the value of a conditional value is required"))?
            {
                conditional_value::Value::Column(name) => {
                    column(table, name, &path)?;
                }
                conditional_value::Value::Constant(value) => {
                    let data_type = self.values.scalar(value, &path)?.data_type();
                    if encoding.scale.is_none() {
                        raw_values(channel, data_type, &path)?;
                    }
                }
                conditional_value::Value::ScaledConstant(value) => {
                    self.values.scalar(value, &path)?;
                }
            }
        }
        if !encoding.selection_field.is_empty() {
            if scale_channel(channel) != channel {
                return Err(fail(
                    path,
                    "the second channel of a pair has no selection field",
                ));
            }
            column(table, &encoding.selection_field, path)?;
        }
        Ok(())
    }

    fn scale(
        &self,
        spec: &proto::ScaleSpec,
        channel: Channel,
        kind: CoordKind,
        value_type: &DataType,
        path: &str,
    ) -> Result<()> {
        let scale_type = (spec.r#type.as_ref())
            .ok_or_else(|| fail(path, "the type of a scale spec is required"))?;
        scale_options(scale_type, value_type, path)?;
        let values = domain_values(scale_type);
        if !takes_values(values, value_type) {
            return Err(fail(
                path,
                format!(
                    "values of type {value_type} are invalid on a {} scale",
                    type_name(scale_type)
                ),
            ));
        }
        let domain = spec
            .domain
            .as_ref()
            .and_then(|domain| domain.domain.as_ref());
        let explicit = match domain {
            None => {
                return Err(fail(
                    format!("{path}.domain"),
                    "the domain of a scale spec is required",
                ))
            }
            Some(domain::Domain::Explicit(value)) => {
                Some(self.explicit(value, scale_type, &format!("{path}.domain.explicit"))?)
            }
            Some(domain::Domain::Summary(summary)) => {
                self.summary(summary, scale_type, &format!("{path}.domain.summary"))?;
                None
            }
            Some(domain::Domain::Data(data)) => {
                self.data(data, scale_type, &format!("{path}.domain.data"))?;
                None
            }
            Some(domain::Domain::Selection(selection)) => {
                let path = format!("{path}.domain.selection");
                self.selection(&selection.selection, &format!("{path}.selection"))?;
                if selection.channel == Channel::Unspecified as i32 {
                    return Err(fail(
                        format!("{path}.channel"),
                        "a selection domain names its channel",
                    ));
                }
                match selection.fallback.as_ref() {
                    Some(selection_domain::Fallback::Explicit(value)) => {
                        Some(self.explicit(value, scale_type, &format!("{path}.explicit"))?)
                    }
                    Some(selection_domain::Fallback::Summary(summary)) => {
                        self.summary(summary, scale_type, &format!("{path}.summary"))?;
                        None
                    }
                    Some(selection_domain::Fallback::Data(data)) => {
                        self.data(data, scale_type, &format!("{path}.data"))?;
                        None
                    }
                    None => {
                        return Err(fail(
                            &path,
                            "the fallback of a selection domain is required",
                        ))
                    }
                }
            }
        };
        if let Some(range) = &spec.range {
            let path = format!("{path}.range");
            match &range.range {
                Some(_) if is_position(channel) => {
                    return Err(fail(
                        &path,
                        "the plot gives the range of a position channel, which sets only `reverse`",
                    ))
                }
                Some(range::Range::Values(value)) => {
                    self.values.list(value, &format!("{path}.values"))?;
                }
                Some(range::Range::Palette(name)) => {
                    named(name, &format!("{path}.palette"), "the name of a palette")?
                }
                None => {}
            }
        }
        if let Some(navigation) = &spec.navigation_limits {
            let path = format!("{path}.navigation_limits");
            let continuous =
                ticked(scale_type) || matches!(scale_type, scale_spec::Type::Custom(_));
            if !(matches!(channel, Channel::X | Channel::Y) && continuous) {
                return Err(fail(
                    &path,
                    "a navigation is valid only on a continuous scale of `x` or `y`",
                ));
            }
            if let Some(navigation_limits::Bounds::Interval(interval)) = &navigation.bounds {
                if !(interval.low.is_finite()
                    && interval.high.is_finite()
                    && interval.low < interval.high)
                {
                    return Err(fail(
                        format!("{path}.interval"),
                        "the bounds of a navigation run from `low` up to `high`",
                    ));
                }
            }
            positive(navigation.min_span, &format!("{path}.min_span"))?;
        }
        if let Some(guide) = spec.guide.as_ref().and_then(|guide| guide.guide.as_ref()) {
            let domain_type = explicit
                .as_ref()
                .map_or(value_type, |array| array.data_type());
            self.guide(
                guide,
                channel,
                kind,
                scale_type,
                domain_type,
                &format!("{path}.guide"),
            )?;
        }
        Ok(())
    }

    /// Checks an explicit domain and returns its values.
    fn explicit(
        &self,
        value: &proto::ValueRef,
        scale_type: &scale_spec::Type,
        path: &str,
    ) -> Result<ArrayRef> {
        let array = self.values.list(value, path)?;
        if !takes_values(domain_values(scale_type), array.data_type()) {
            return Err(fail(
                path,
                format!(
                    "a domain of type {} is invalid on a {} scale",
                    array.data_type(),
                    type_name(scale_type)
                ),
            ));
        }
        let fit = FitKind::of(scale_type);
        if fit == Some(FitKind::Interval) && array.len() != 2 {
            return Err(fail(
                path,
                format!(
                    "the explicit domain of a {} scale holds its two ends, not {} values",
                    type_name(scale_type),
                    array.len()
                ),
            ));
        }
        if fit == Some(FitKind::Interval) || matches!(scale_type, scale_spec::Type::Threshold(_)) {
            ascends(&array, path)?;
        }
        Ok(array)
    }

    /// Checks the layout of a summary domain against the kind of its scale.
    /// Checks a summary domain: its summary and its fit.
    fn summary(
        &self,
        summary: &proto::SummaryDomain,
        scale_type: &scale_spec::Type,
        path: &str,
    ) -> Result<()> {
        if let Some(fit) = &summary.fit {
            self.fit(fit, scale_type, &format!("{path}.fit"))?;
        }
        let path = format!("{path}.value");
        let path = path.as_str();
        let summary = self.values.scalar(summary.value.as_ref(), path)?;
        let DataType::Struct(fields) = summary.data_type() else {
            return Err(fail(
                path,
                format!(
                    "a summary is a struct, not a value of type {}",
                    summary.data_type()
                ),
            ));
        };
        let kind = summary_kind(fields).ok_or_else(|| {
            fail(
                path,
                "the fields of a summary are those of one of its three layouts",
            )
        })?;
        let expected = match FitKind::of(scale_type) {
            Some(FitKind::Explicit) => {
                return Err(fail(
                    path,
                    "a summary domain is invalid on a threshold scale",
                ));
            }
            Some(kind) => kind,
            None => return Ok(()),
        };
        if kind != expected {
            return Err(fail(
                path,
                format!(
                    "the summary does not have the layout of a {} scale",
                    type_name(scale_type)
                ),
            ));
        }
        Ok(())
    }

    /// Checks a data domain: its scale summarizes data, and its fit.
    fn data(
        &self,
        data: &proto::DataDomain,
        scale_type: &scale_spec::Type,
        path: &str,
    ) -> Result<()> {
        data_domain(scale_type, path)?;
        match &data.fit {
            Some(fit) => self.fit(fit, scale_type, &format!("{path}.fit")),
            None => Ok(()),
        }
    }

    fn fit(&self, fit: &proto::DomainFit, scale_type: &scale_spec::Type, path: &str) -> Result<()> {
        let kind = FitKind::of(scale_type);
        match fit.fit.as_ref() {
            Some(domain_fit::Fit::Interval(interval))
                if kind.is_none_or(|kind| kind == FitKind::Interval) =>
            {
                let path = format!("{path}.interval");
                positive(interval.nice, &format!("{path}.nice"))?;
                if let Some(empty) = &interval.empty {
                    self.values.list(empty, &format!("{path}.empty"))?;
                }
                Ok(())
            }
            Some(domain_fit::Fit::Categories(categories))
                if kind.is_none_or(|kind| kind == FitKind::Categories) =>
            {
                let path = format!("{path}.categories");
                match categories.order.as_ref() {
                    Some(category_fit::Order::Sort(_)) => Ok(()),
                    Some(category_fit::Order::Preferred(value)) => {
                        if matches!(scale_type, scale_spec::Type::NestedBand(_)) {
                            return Err(fail(
                                &path,
                                "a preferred order is invalid on a nested band scale",
                            ));
                        }
                        self.values
                            .list(value, &format!("{path}.preferred"))
                            .map(|_| ())
                    }
                    // An unset order is ascending, the default of a category fit.
                    None => Ok(()),
                }
            }
            Some(_) => Err(fail(
                path,
                format!(
                    "the fit is not of the kind that a {} scale fits",
                    type_name(scale_type)
                ),
            )),
            // A fit without a kind is the fit of the scale's kind with no field set.
            None => Ok(()),
        }
    }

    fn guide(
        &self,
        guide: &guide::Guide,
        channel: Channel,
        kind: CoordKind,
        scale_type: &scale_spec::Type,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        match guide {
            guide::Guide::Axis(axis) => {
                if kind != CoordKind::Cartesian || !matches!(channel, Channel::X | Channel::Y) {
                    return Err(fail(
                        path,
                        "an axis is the guide of `x` or `y` of a Cartesian plot",
                    ));
                }
                let path = format!("{path}.axis");
                classes(&axis.classes, &path)?;
                self.axis(axis, scale_type, domain_type, &path)
            }
            guide::Guide::PolarAxis(axis) => {
                let path = format!("{path}.polar_axis");
                if !matches!(channel, Channel::Theta | Channel::Radius) {
                    return Err(fail(
                        &path,
                        "a polar axis is the guide of `theta` or `radius`",
                    ));
                }
                if axis.angle.is_some() && channel != Channel::Radius {
                    return Err(fail(
                        format!("{path}.angle"),
                        "an angle is valid only on a polar axis of `radius`",
                    ));
                }
                classes(&axis.classes, &path)?;
                self.labels(axis.labels.as_ref(), domain_type, &path)?;
                self.ticks(axis.ticks.as_ref(), scale_type, domain_type, &path)
            }
            guide::Guide::Legend(legend) => {
                let path = format!("{path}.legend");
                classes(&legend.classes, &path)?;
                if is_position(channel) {
                    return Err(fail(
                        &path,
                        "a legend is the guide of a channel other than a position",
                    ));
                }
                if !ticked(scale_type) && legend.entries.is_some() {
                    return Err(fail(
                        &path,
                        "only the legend of a linear, log, pow, symlog, or time scale takes a tick \
                         count or values",
                    ));
                }
                match &legend.entries {
                    Some(legend::Entries::TickCount(count)) => {
                        positive(Some(*count), &format!("{path}.tick_count"))?
                    }
                    Some(legend::Entries::Values(values)) => {
                        self.domain_values(values, domain_type, &format!("{path}.values"))?
                    }
                    None => {}
                }
                self.labels(legend.labels.as_ref(), domain_type, &path)
            }
            guide::Guide::Colorbar(colorbar) => {
                let path = format!("{path}.colorbar");
                classes(&colorbar.classes, &path)?;
                if !matches!(channel, Channel::Color | Channel::Fill | Channel::Stroke)
                    || !ticked(scale_type)
                {
                    return Err(fail(
                        &path,
                        "a colorbar is the guide of a linear, log, pow, symlog, or time scale of a \
                         color channel",
                    ));
                }
                match &colorbar.axis {
                    Some(axis) => {
                        self.colorbar_axis(axis, scale_type, domain_type, &format!("{path}.axis"))
                    }
                    None => Ok(()),
                }
            }
            guide::Guide::None(_) => Ok(()),
        }
    }

    fn axis(
        &self,
        axis: &proto::Axis,
        scale_type: &scale_spec::Type,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        self.labels(axis.labels.as_ref(), domain_type, path)?;
        self.ticks(axis.ticks.as_ref(), scale_type, domain_type, path)?;
        self.levels(&axis.levels, scale_type, domain_type, path)
    }

    /// Checks the levels of an axis: valid only on the axis of a nested band scale, with at most
    /// one for each level of its paths, and the pattern of each in the syntax of its values.
    fn levels(
        &self,
        levels: &[proto::NestedAxisLevel],
        scale_type: &scale_spec::Type,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        if levels.is_empty() {
            return Ok(());
        }
        let path = format!("{path}.levels");
        if !matches!(scale_type, scale_spec::Type::NestedBand(_)) {
            return Err(fail(
                &path,
                "levels are valid only on the axis of a nested band scale",
            ));
        }
        // The domain of a nested band scale holds structs, which the check of its domain checked.
        let DataType::Struct(fields) = dictionary_values(domain_type) else {
            return Ok(());
        };
        if levels.len() > fields.len() {
            return Err(fail(
                &path,
                format!(
                    "the axis has {} levels, and the paths of its scale have {}",
                    levels.len(),
                    fields.len()
                ),
            ));
        }
        for (index, (level, field)) in levels.iter().zip(fields).enumerate() {
            self.pattern(
                &level.format,
                field.data_type(),
                &format!("{path}[{index}].format"),
            )?;
        }
        Ok(())
    }

    fn colorbar_axis(
        &self,
        axis: &proto::ColorbarAxis,
        scale_type: &scale_spec::Type,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        self.labels(axis.labels.as_ref(), domain_type, path)?;
        self.ticks(axis.ticks.as_ref(), scale_type, domain_type, path)
    }

    fn labels(
        &self,
        labels: Option<&proto::GuideLabels>,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        match labels {
            Some(labels) => self.pattern(
                &labels.format,
                domain_type,
                &format!("{path}.labels.format"),
            ),
            None => Ok(()),
        }
    }

    /// Checks the ticks of an axis. On the axis of a band or a point scale, only a list of ticks is
    /// valid, and on the axis of a nested band scale, which ticks at the bands of its leaf, the
    /// ticks of the scale.
    fn ticks(
        &self,
        ticks: Option<&proto::GuideTicks>,
        scale_type: &scale_spec::Type,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        let Some(ticks) = ticks else { return Ok(()) };
        let path = format!("{path}.ticks");
        non_negative(ticks.length.map(f64::from), &format!("{path}.length"))?;
        use scale_spec::Type;
        match (scale_type, &ticks.at) {
            (Type::NestedBand(_), Some(at)) => {
                return Err(fail(
                    format!("{path}.{}", at_name(at)),
                    "the axis of a nested band scale ticks at the bands of its leaf, so its ticks \
                     stand nowhere else",
                ))
            }
            (
                Type::Band(_) | Type::Point(_),
                Some(at @ (guide_ticks::At::Count(_) | guide_ticks::At::Interval(_))),
            ) => {
                return Err(fail(
                    format!("{path}.{}", at_name(at)),
                    "only a list of ticks is valid on the axis of a band or a point scale",
                ))
            }
            _ => {}
        }
        match &ticks.at {
            Some(guide_ticks::At::Count(count)) => positive(Some(*count), &format!("{path}.count")),
            Some(guide_ticks::At::Values(values)) => {
                self.domain_values(values, domain_type, &format!("{path}.values"))
            }
            Some(guide_ticks::At::Interval(interval)) => {
                tick_interval(interval, &format!("{path}.interval"))
            }
            None => Ok(()),
        }
    }

    /// Checks tick or entry values, which are in the type of the scale's domain.
    fn domain_values(
        &self,
        value: &proto::ValueRef,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        let values = self.values.list(value, path)?;
        if !compatible(values.data_type(), domain_type) {
            return Err(fail(
                path,
                format!(
                    "the values have type {}, and the scale's domain has type {domain_type}",
                    values.data_type()
                ),
            ));
        }
        Ok(())
    }
}

/// Checks the options of a scale type, whose values have type `value_type`.
fn scale_options(scale_type: &scale_spec::Type, value_type: &DataType, path: &str) -> Result<()> {
    use scale_spec::Type;
    let fraction = |value: Option<f64>, at: &str| within(value, 0.0, 1.0, &format!("{path}.{at}"));
    match scale_type {
        Type::Log(log) => {
            if log
                .base
                .is_some_and(|base| !(base.is_finite() && base > 0.0 && base != 1.0))
            {
                return Err(fail(
                    format!("{path}.log.base"),
                    "the base of a log scale is more than 0 and not 1",
                ));
            }
        }
        Type::Pow(pow) => {
            if pow
                .exponent
                .is_some_and(|exponent| !(exponent.is_finite() && exponent != 0.0))
            {
                return Err(fail(
                    format!("{path}.pow.exponent"),
                    "the exponent of a pow scale is finite and not 0",
                ));
            }
        }
        Type::Symlog(symlog) => positive(symlog.constant, &format!("{path}.symlog.constant"))?,
        Type::Band(band) => {
            fraction(band.align, "band.align")?;
            fraction(band.band, "band.band")?;
            fraction(band.padding_inner, "band.padding_inner")?;
            non_negative(band.padding_outer, &format!("{path}.band.padding_outer"))?;
            non_negative(
                band.clip_padding_lower,
                &format!("{path}.band.clip_padding_lower"),
            )?;
            non_negative(
                band.clip_padding_upper,
                &format!("{path}.band.clip_padding_upper"),
            )?;
        }
        Type::Point(point) => {
            fraction(point.align, "point.align")?;
            non_negative(point.padding, &format!("{path}.point.padding"))?;
            non_negative(
                point.clip_padding_lower,
                &format!("{path}.point.clip_padding_lower"),
            )?;
            non_negative(
                point.clip_padding_upper,
                &format!("{path}.point.clip_padding_upper"),
            )?;
        }
        Type::NestedBand(nested) => {
            let path = format!("{path}.nested_band");
            fraction(nested.align, "nested_band.align")?;
            fraction(nested.band, "nested_band.band")?;
            let levels = match dictionary_values(value_type) {
                DataType::Struct(fields) => fields.len(),
                _ => 0,
            };
            if nested.level.is_some_and(|level| level as usize >= levels) {
                return Err(fail(
                    format!("{path}.level"),
                    format!("the paths have {levels} levels"),
                ));
            }
            let per_level = |count: usize, at: &str| -> Result<()> {
                if count > 1 && count != levels {
                    return Err(fail(
                        format!("{path}.{at}"),
                        format!("one entry sets every level, or one entry for each of the {levels} levels"),
                    ));
                }
                Ok(())
            };
            per_level(nested.nest_scopes.len(), "nest_scopes")?;
            per_level(nested.padding_inner.len(), "padding_inner")?;
            per_level(nested.padding_outer.len(), "padding_outer")?;
            for (index, padding) in nested
                .padding_inner
                .iter()
                .chain(&nested.padding_outer)
                .enumerate()
            {
                non_negative(Some(*padding), &format!("{path}.padding[{index}]"))?;
            }
            non_negative(
                nested.clip_padding_lower,
                &format!("{path}.clip_padding_lower"),
            )?;
            non_negative(
                nested.clip_padding_upper,
                &format!("{path}.clip_padding_upper"),
            )?;
        }
        Type::Custom(custom) => {
            named(
                &custom.r#type,
                &format!("{path}.custom.type"),
                "the type of a custom scale",
            )?;
            keyed(
                &custom.options,
                |option| option.name.clone(),
                |a, b| a == b,
                &format!("{path}.custom.options"),
                "options",
            )?;
        }
        Type::Linear(_)
        | Type::Time(_)
        | Type::Ordinal(_)
        | Type::Quantile(_)
        | Type::Quantize(_)
        | Type::Threshold(_) => {}
    }
    Ok(())
}

fn data_domain(scale_type: &scale_spec::Type, path: &str) -> Result<()> {
    if matches!(scale_type, scale_spec::Type::Threshold(_)) {
        return Err(fail(
            path,
            "a data domain is invalid on a threshold scale, whose domain is explicit",
        ));
    }
    Ok(())
}

/// The kind of a summary's layout, by its fields.
fn summary_kind(fields: &Fields) -> Option<FitKind> {
    let names: Vec<&str> = fields.iter().map(|field| field.name().as_str()).collect();
    let floats = |name: &str| {
        fields.find(name).is_some_and(|(_, field)| {
            matches!(field.data_type(), DataType::List(item) if item.data_type() == &DataType::Float64)
        })
    };
    match names.as_slice() {
        ["lower_values", "lower_extents", "upper_values", "upper_extents"]
            if names.iter().all(|name| floats(name)) =>
        {
            Some(FitKind::Interval)
        }
        ["categories", "lower_extents", "upper_extents"]
            if floats("lower_extents")
                && floats("upper_extents")
                && matches!(fields[0].data_type(), DataType::List(_)) =>
        {
            Some(FitKind::Categories)
        }
        ["values"] if floats("values") => Some(FitKind::Sample),
        _ => None,
    }
}

fn tick_interval(interval: &proto::TickInterval, path: &str) -> Result<()> {
    use proto::{calendar_ticks::Step, tick_interval::Interval};
    match interval.interval.as_ref() {
        Some(Interval::Numeric(numeric)) => {
            if !numeric.start.is_finite() {
                return Err(fail(
                    format!("{path}.numeric.start"),
                    "the start of ticks is finite",
                ));
            }
            positive(Some(numeric.step), &format!("{path}.numeric.step"))
        }
        Some(Interval::Calendar(calendar)) => match calendar.step {
            Some(Step::Months(step) | Step::Days(step)) if step <= 0 => Err(fail(
                format!("{path}.calendar"),
                "a calendar step is more than 0",
            )),
            Some(Step::Milliseconds(step)) if step <= 0 => Err(fail(
                format!("{path}.calendar"),
                "a calendar step is more than 0",
            )),
            Some(_) => Ok(()),
            None => Err(fail(
                format!("{path}.calendar"),
                "calendar ticks set their step",
            )),
        },
        None => Err(fail(path, "a tick interval sets its kind")),
    }
}

/// The text properties of a layer whose mark is a text.
struct TextProps<'a> {
    format: &'a str,
    width_mode: i32,
    wrap: Option<bool>,
    max_lines: Option<u32>,
    ellipsis: Option<bool>,
}

fn text_props(layer: LayerRef<'_>) -> Option<TextProps<'_>> {
    use proto::{cartesian_layer, geo_layer};
    match layer {
        LayerRef::Cartesian(proto::CartesianLayer {
            mark: Some(cartesian_layer::Mark::Text(text)),
            ..
        }) => Some(TextProps {
            format: &text.format,
            width_mode: text.width_mode,
            wrap: text.wrap,
            max_lines: text.max_lines,
            ellipsis: text.ellipsis,
        }),
        LayerRef::Polar(proto::PolarLayer {
            mark: Some(polar_layer::Mark::Text(text)),
            ..
        }) => Some(TextProps {
            format: &text.format,
            width_mode: text.width_mode,
            wrap: text.wrap,
            max_lines: text.max_lines,
            ellipsis: text.ellipsis,
        }),
        LayerRef::Geo(proto::GeoLayer {
            mark: Some(geo_layer::Mark::Text(text)),
            ..
        }) => Some(TextProps {
            format: &text.format,
            width_mode: text.width_mode,
            wrap: text.wrap,
            max_lines: text.max_lines,
            ellipsis: text.ellipsis,
        }),
        _ => None,
    }
}

impl TextProps<'_> {
    /// Checks the wrapping of a text: it keeps a line or more, its width mode and its wrap need a
    /// `width`, and its ellipsis needs a `width` or a most lines, which cut the text.
    fn check(&self, width: bool, path: &str) -> Result<()> {
        if self.max_lines == Some(0) {
            return Err(fail(
                format!("{path}.max_lines"),
                "a text keeps one line or more",
            ));
        }
        if !width
            && (self.width_mode != proto::TextWidthMode::Unspecified as i32 || self.wrap.is_some())
        {
            return Err(fail(
                path,
                "the width mode and the wrap of a text apply to its `width`, which it does not encode",
            ));
        }
        if !width && self.max_lines.is_none() && self.ellipsis.is_some() {
            return Err(fail(
                path,
                "the ellipsis of a text marks the cut of its `width` or its most lines, and it has \
                 neither",
            ));
        }
        Ok(())
    }
}

/// Checks that a rect without the second channel of a position spans the band of that position
/// on a band scale.
fn rect_spans(mark: &dyn MarkMessage, kind: CoordKind, path: &str) -> Result<()> {
    for [primary, pair] in kind.pairs().iter().copied() {
        if let (Some(position), None) = (mark.encoding(primary), mark.encoding(pair)) {
            if !is_band(position) {
                return Err(fail(
                    path,
                    format!(
                        "a rect with `{}` and without `{}` spans a band of a band scale",
                        primary.field_name(),
                        pair.field_name()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn is_band(encoding: &proto::Encoding) -> bool {
    matches!(
        encoding
            .scale
            .as_ref()
            .and_then(|scale| scale.r#type.as_ref()),
        Some(scale_spec::Type::Band(_))
    )
}

/// Checks that a bar in a polar plot without the second channel of its key, `theta` for a
/// vertical bar and `radius` for a horizontal one, spans a band of its key on a band scale.
fn polar_bar_spans(mark: &dyn MarkMessage, bar: &proto::PolarBarMark, path: &str) -> Result<()> {
    let continuous = |channel| {
        (mark.encoding(channel))
            .and_then(|encoding| encoding.scale.as_ref())
            .and_then(|scale| scale.r#type.as_ref())
            .is_some_and(|scale_type| FitKind::of(scale_type) == Some(FitKind::Interval))
    };
    let horizontal = match proto::Orient::try_from(bar.orient) {
        Ok(proto::Orient::Horizontal) => true,
        Ok(proto::Orient::Vertical) => false,
        _ => continuous(Channel::Theta) && !continuous(Channel::Radius),
    };
    let (key, key2) = match horizontal {
        true => (Channel::Radius, Channel::Radius2),
        false => (Channel::Theta, Channel::Theta2),
    };
    if let (Some(position), None) = (mark.encoding(key), mark.encoding(key2)) {
        if !is_band(position) {
            return Err(fail(
                path,
                format!(
                    "a bar in a polar plot with `{}` and without `{}` spans a band of a band scale",
                    key.field_name(),
                    key2.field_name()
                ),
            ));
        }
    }
    Ok(())
}

/// Checks that the channels of a line, a trail, or an area that take one value for each series
/// hold one value in each series that the `detail` and the color channels split.
fn one_value_per_series(mark: &dyn MarkMessage, table: &RecordBatch, path: &str) -> Result<()> {
    let per_series: &[Channel] = match mark.mark_type() {
        MarkType::Line => &[
            Channel::Opacity,
            Channel::StrokeOpacity,
            Channel::StrokeWidth,
            Channel::StrokeDash,
        ],
        MarkType::Trail => &[Channel::Opacity],
        MarkType::Area => &[
            Channel::Opacity,
            Channel::FillOpacity,
            Channel::StrokeOpacity,
            Channel::StrokeWidth,
            Channel::StrokeDash,
            Channel::Pattern,
        ],
        _ => return Ok(()),
    };
    let column_of = |channel: Channel| match &mark.encoding(channel)?.value {
        Some(encoding::Value::Column(name)) => table.column_by_name(name).cloned(),
        _ => None,
    };
    let checked: Vec<(Channel, ArrayRef)> = (per_series.iter())
        .filter_map(|channel| column_of(*channel).map(|column| (*channel, column)))
        .collect();
    if checked.is_empty() || table.num_rows() == 0 {
        return Ok(());
    }
    let keys: Vec<ArrayRef> = [
        Channel::Color,
        Channel::Fill,
        Channel::Stroke,
        Channel::Detail,
    ]
    .into_iter()
    .filter_map(column_of)
    .collect();
    let rows = |columns: &[ArrayRef]| {
        let converter = RowConverter::new(
            (columns.iter())
                .map(|column| SortField::new(column.data_type().clone()))
                .collect(),
        )?;
        converter.convert_columns(columns)
    };
    let key_rows = match keys.is_empty() {
        true => None,
        false => Some(rows(&keys).map_err(|error| fail(path, error.to_string()))?),
    };
    for (channel, column) in checked {
        let values =
            rows(std::slice::from_ref(&column)).map_err(|error| fail(path, error.to_string()))?;
        let mut first: HashMap<&[u8], &[u8]> = HashMap::new();
        for row in 0..table.num_rows() {
            let key = key_rows
                .as_ref()
                .map_or(&[][..], |rows| rows.row(row).data());
            let value = values.row(row).data();
            if *first.entry(key).or_insert(value) != value {
                return Err(fail(
                    format!("{path}.{}", channel.field_name()),
                    format!(
                        "`{}` takes one value for each series of a `{}` mark, and its column changes \
                         within a series",
                        channel.field_name(),
                        mark.name()
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Checks values without a scale against the form of their channel.
fn raw_values(channel: Channel, data_type: &DataType, path: &str) -> Result<()> {
    let ok = match channel {
        Channel::Color
        | Channel::Fill
        | Channel::Stroke
        | Channel::Shape
        | Channel::Pattern
        | Channel::Path
        | Channel::FontStyle => is_string(data_type),
        Channel::FontWeight => data_type.is_numeric() || is_string(data_type),
        Channel::Opacity
        | Channel::FillOpacity
        | Channel::StrokeOpacity
        | Channel::StrokeWidth
        | Channel::Size
        | Channel::Angle
        | Channel::Theta
        | Channel::Radius
        | Channel::Longitude
        | Channel::Latitude
        | Channel::Longitude2
        | Channel::Latitude2
        | Channel::Width
        | Channel::Height => data_type.is_numeric(),
        Channel::StrokeDash => {
            is_string(data_type)
                || matches!(
                    data_type,
                    DataType::List(field) | DataType::LargeList(field) | DataType::FixedSizeList(field, _)
                        if field.data_type().is_numeric()
                )
        }
        Channel::Image => {
            is_string(data_type)
                || matches!(data_type, DataType::Struct(fields)
                    if ["width", "height", "data"].iter().all(|name| fields.find(name).is_some()))
        }
        _ => true,
    };
    if !ok {
        return Err(fail(
            path,
            format!(
                "`{}` values without a scale cannot have type {data_type}",
                channel.field_name()
            ),
        ));
    }
    Ok(())
}

/// Whether values of type `values` stand for values of a domain of type `domain`: numbers for
/// numbers, strings for strings, and otherwise one type.
fn compatible(values: &DataType, domain: &DataType) -> bool {
    let (values, domain) = (dictionary_values(values), dictionary_values(domain));
    (values.is_numeric() && domain.is_numeric())
        || (is_string(values) && is_string(domain))
        || values == domain
}

/// Fails when a value of `array` is less than the one before it.
fn ascends(array: &ArrayRef, path: &str) -> Result<()> {
    if array.len() < 2 {
        return Ok(());
    }
    let next = array.slice(1, array.len() - 1);
    let previous = array.slice(0, array.len() - 1);
    let less = lt(&next, &previous).map_err(|error| fail(path, error.to_string()))?;
    if less.true_count() > 0 {
        return Err(fail(path, "the values are in ascending order"));
    }
    Ok(())
}

fn column<'b>(table: &'b RecordBatch, name: &str, path: &str) -> Result<&'b ArrayRef> {
    (table.column_by_name(name))
        .ok_or_else(|| fail(path, format!("the layer's table has no column `{name}`")))
}

pub(crate) fn is_position(channel: Channel) -> bool {
    matches!(
        channel,
        Channel::X
            | Channel::Y
            | Channel::X2
            | Channel::Y2
            | Channel::Longitude
            | Channel::Latitude
            | Channel::Longitude2
            | Channel::Latitude2
            | Channel::Geometry
            | Channel::Theta
            | Channel::Radius
            | Channel::Theta2
            | Channel::Radius2
    )
}

/// Whether a channel can take a scale: not the second channels of pairs, which follow the first,
/// and not the channels of raw values.
pub(crate) fn takes_scale(channel: Channel) -> bool {
    !matches!(
        channel,
        Channel::Unspecified
            | Channel::X2
            | Channel::Y2
            | Channel::Theta2
            | Channel::Radius2
            | Channel::Longitude
            | Channel::Latitude
            | Channel::Longitude2
            | Channel::Latitude2
            | Channel::Geometry
            | Channel::Text
            | Channel::Image
            | Channel::Path
            | Channel::FontWeight
            | Channel::FontStyle
            | Channel::Width
            | Channel::Height
            | Channel::Detail
    )
}

/// The name of the member of `GuideTicks.at` that `at` holds.
fn at_name(at: &guide_ticks::At) -> &'static str {
    match at {
        guide_ticks::At::Count(_) => "count",
        guide_ticks::At::Interval(_) => "interval",
        guide_ticks::At::Values(_) => "values",
    }
}
