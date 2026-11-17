//! The rules of marks and their encodings: the channels that a mark needs, the values and the
//! scales of encodings, the domains, fits, ranges, axes, and legends of scales, and the patterns
//! that format their labels.

use std::collections::HashMap;

use arrow::{
    array::{Array, ArrayRef},
    compute::kernels::cmp::lt,
    datatypes::{DataType, Fields},
    record_batch::RecordBatch,
    row::{RowConverter, SortField},
};

use super::{
    classes, dictionary_values, fail, is_string, named, non_negative, positive, title, within,
    Checker, Result, REQUIRED,
};
use crate::{
    layer::{CoordKind, LayerRef, MarkMessage, MarkType},
    lump,
    proto::{
        self, axis, conditional_value, encoding, legend, navigation_limits, polar_layer, scale,
        selection_domain, Channel, LegendForm,
    },
    resolve::scale_channel,
    span,
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
    pub fn of(scale_type: &scale::Type) -> Option<Self> {
        use scale::Type;
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

fn domain_values(scale_type: &scale::Type) -> DomainValues {
    use scale::Type;
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
fn type_name(scale_type: &scale::Type) -> &'static str {
    use scale::Type;
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
fn ticked(scale_type: &scale::Type) -> bool {
    use scale::Type;
    matches!(
        scale_type,
        Type::Linear(_) | Type::Log(_) | Type::Pow(_) | Type::Symlog(_) | Type::Time(_)
    )
}

impl Checker<'_> {
    /// Checks a layer's mark against its table.
    pub(super) fn layer(&self, layer: LayerRef, table: &RecordBatch, path: &str) -> Result<()> {
        let mark = layer.mark().expect(REQUIRED);
        let mark_path = format!("{path}.{}", mark.name());
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
            if first != channel && takes_scale(first) {
                let first_scaled = mark.encoding(first).is_some_and(scaled);
                if scaled(encoding) != first_scaled {
                    return Err(fail(
                        &path,
                        format!(
                            "`{}` goes through the scale of `{}`, so it is a `scaled_` value \
                             exactly when `{}` is",
                            channel.field_name(),
                            first.field_name(),
                            first.field_name()
                        ),
                    ));
                }
            }
            spans(mark, channel, encoding, table, &path)?;
            self.encoding(encoding, channel, layer.kind(), table, &path)?;
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
        if let LayerRef::Polar(proto::PolarLayer {
            mark: Some(polar_layer::Mark::Text(text)),
            ..
        }) = layer
        {
            if text.orient != 0 && text.angle.is_some() {
                return Err(fail(
                    format!("{mark_path}.angle"),
                    "a polar text turns by its `orient` or by its `angle`, not both",
                ));
            }
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

    /// Checks that each column of lumps that `encoding` reads goes through a band, a point, or an
    /// ordinal scale, which places the Other group after the kept categories.
    fn lumps_are_categorical(
        &self,
        encoding: &proto::Encoding,
        table: &RecordBatch,
        path: &str,
    ) -> Result<()> {
        let is_lump =
            |name: &str| (table.column_by_name(name)).is_some_and(|c| lump::is_lump(c.data_type()));
        let direct =
            (encoding.kind.iter())
                .filter_map(|kind| match kind {
                    encoding::Kind::DirectField(name) => Some(name),
                    _ => None,
                })
                .chain(encoding.conditional.iter().filter_map(
                    |conditional| match &conditional.kind {
                        Some(conditional_value::Kind::DirectField(name)) => Some(name),
                        _ => None,
                    },
                ));
        for name in direct {
            if is_lump(name) {
                return Err(fail(
                    path,
                    format!(
                        "`{name}` is a column of lumps, which goes through a band, a point, or an \
                         ordinal scale, so it is a `scaled_field`"
                    ),
                ));
            }
        }
        let scaled_lump = match &encoding.kind {
            Some(encoding::Kind::ScaledField(name)) => is_lump(name).then_some(name),
            _ => None,
        };
        let categorical = (encoding
            .scale
            .as_ref()
            .and_then(|scale| scale.r#type.as_ref()))
        .is_none_or(|scale_type| {
            matches!(
                scale_type,
                scale::Type::Band(_) | scale::Type::Point(_) | scale::Type::Ordinal(_)
            )
        });
        if let Some(name) = scaled_lump.filter(|_| !categorical) {
            return Err(fail(
                format!("{path}.scale"),
                format!(
                    "`{name}` is a column of lumps, which goes through a band, a point, or an \
                     ordinal scale"
                ),
            ));
        }
        Ok(())
    }

    /// The type of the values of `encoding`: of its column, or of its constant.
    fn value_type(
        &self,
        encoding: &proto::Encoding,
        table: &RecordBatch,
        path: &str,
    ) -> Result<DataType> {
        Ok(match encoding.kind.as_ref() {
            Some(encoding::Kind::DirectField(name) | encoding::Kind::ScaledField(name)) => {
                lump::values_type(&span::values_type(column(table, name, path)?.data_type()))
            }
            Some(encoding::Kind::DirectValue(value) | encoding::Kind::ScaledValue(value)) => {
                self.values.scalar(value, path)?.data_type().clone()
            }
            None => unreachable!("{REQUIRED}"),
        })
    }

    /// The type of the values of the first `scaled_` conditional value of `encoding`, which has
    /// one.
    fn scaled_conditional_type(
        &self,
        encoding: &proto::Encoding,
        table: &RecordBatch,
        path: &str,
    ) -> Result<DataType> {
        for (index, conditional) in encoding.conditional.iter().enumerate() {
            let path = format!("{path}.conditional[{index}]");
            match &conditional.kind {
                Some(conditional_value::Kind::ScaledField(name)) => {
                    return Ok(column(table, name, &path)?.data_type().clone())
                }
                Some(conditional_value::Kind::ScaledValue(value)) => {
                    return Ok(self.values.scalar(value, &path)?.data_type().clone())
                }
                _ => {}
            }
        }
        unreachable!("an encoding with a scale has a scaled value")
    }

    fn encoding(
        &self,
        encoding: &proto::Encoding,
        channel: Channel,
        kind: CoordKind,
        table: &RecordBatch,
        path: &str,
    ) -> Result<()> {
        let value = encoding.kind.as_ref().expect(REQUIRED);
        let value_type = self.value_type(encoding, table, path)?;
        if channel == Channel::Geometry {
            let geoarrow = match value {
                encoding::Kind::DirectField(name) => (table.schema().field_with_name(name).ok())
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
        self.lumps_are_categorical(encoding, table, path)?;
        let scaled = scaled(encoding);
        let paired = scale_channel(channel) != channel;
        if scaled && !paired && !takes_scale(channel) {
            return Err(fail(
                path,
                format!(
                    "`{}` takes no scale, so its values are a `direct_field` or a `direct_value`",
                    channel.field_name()
                ),
            ));
        }
        match &encoding.scale {
            Some(_) if !takes_scale(channel) => {
                return Err(fail(
                    path,
                    format!("`{}` never has a scale", channel.field_name()),
                ))
            }
            Some(_) if !scaled => {
                return Err(fail(
                    path,
                    "an encoding without a `scaled_field` or a `scaled_value` has no scale",
                ))
            }
            None if scaled && !paired => {
                return Err(fail(
                    path,
                    "a `scaled_field` or a `scaled_value` goes through the channel's scale",
                ))
            }
            _ => {}
        }
        if !is_scaled(value) {
            raw_values(channel, &value_type, path)?;
        }
        if encoding.axis.is_some() && !(is_position(channel) && takes_scale(channel)) {
            return Err(fail(
                format!("{path}.axis"),
                "an axis is valid only on a position with a scale of its own: `x`, `y`, `theta`, \
                 or `radius`",
            ));
        }
        if encoding.legend.is_some() && is_position(channel) {
            return Err(fail(
                format!("{path}.legend"),
                "a legend is valid only on a channel other than a position",
            ));
        }
        match &encoding.scale {
            Some(scale) => {
                // The scale takes the type of the encoding's value when it is scaled, and
                // otherwise of the first scaled conditional value.
                let scaled_type = match is_scaled(value) {
                    true => value_type.clone(),
                    false => self.scaled_conditional_type(encoding, table, path)?,
                };
                let domain_type =
                    self.scale(scale, channel, &scaled_type, &format!("{path}.scale"))?;
                let scale_type = scale.r#type.as_ref().expect("`scale` checked the type");
                if let Some(axis) = &encoding.axis {
                    let path = format!("{path}.axis");
                    self.axis(axis, channel, kind, scale_type, &domain_type, &path)?;
                }
                if let Some(legend) = &encoding.legend {
                    let path = format!("{path}.legend");
                    self.legend(legend, channel, scale_type, &domain_type, &path)?;
                }
            }
            None => {
                for (field, set) in [
                    ("axis", encoding.axis.is_some()),
                    ("legend", encoding.legend.is_some()),
                ] {
                    if set {
                        return Err(fail(
                            format!("{path}.{field}"),
                            format!("an encoding without a scale has no {field}"),
                        ));
                    }
                }
            }
        }
        band_placement(encoding, channel, &value_type, path)?;
        if !encoding.conditional.is_empty()
            && (is_position(channel) || matches!(channel, Channel::Detail | Channel::Order))
        {
            return Err(fail(
                path,
                "the position channels, `detail`, and `order` have no conditional values",
            ));
        }
        for (index, conditional) in encoding.conditional.iter().enumerate() {
            let path = format!("{path}.conditional[{index}]");
            let predicate = conditional.predicate.as_ref().expect(REQUIRED);
            self.predicate(predicate, &format!("{path}.predicate"))?;
            match conditional.kind.as_ref().expect(REQUIRED) {
                conditional_value::Kind::DirectField(name) => {
                    raw_values(
                        channel,
                        &span::values_type(column(table, name, &path)?.data_type()),
                        &path,
                    )?;
                }
                conditional_value::Kind::ScaledField(name) => {
                    column(table, name, &path)?;
                }
                conditional_value::Kind::DirectValue(value) => {
                    raw_values(
                        channel,
                        self.values.scalar(value, &path)?.data_type(),
                        &path,
                    )?;
                }
                conditional_value::Kind::ScaledValue(value) => {
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

    /// Checks a scale's spec and returns the type of its domain's values: of its explicit domain,
    /// or otherwise of the encoding's values.
    fn scale(
        &self,
        spec: &proto::Scale,
        channel: Channel,
        value_type: &DataType,
        path: &str,
    ) -> Result<DataType> {
        let scale_type =
            (spec.r#type.as_ref()).ok_or_else(|| fail(path, "the type of a scale is required"))?;
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
        let explicit = match &spec.domain {
            None => {
                data_domain(scale_type, &format!("{path}.domain"))?;
                None
            }
            Some(scale::Domain::Explicit(value)) => {
                Some(self.explicit(value, scale_type, &format!("{path}.explicit"))?)
            }
            Some(scale::Domain::Summary(summary)) => {
                self.summary(summary, scale_type, &format!("{path}.summary"))?;
                None
            }
            Some(scale::Domain::Selection(selection)) => {
                let path = format!("{path}.selection");
                self.selection(&selection.selection, &format!("{path}.selection"))?;
                match selection.fallback.as_ref() {
                    Some(selection_domain::Fallback::Explicit(value)) => {
                        Some(self.explicit(value, scale_type, &format!("{path}.explicit"))?)
                    }
                    Some(selection_domain::Fallback::Summary(summary)) => {
                        self.summary(summary, scale_type, &format!("{path}.summary"))?;
                        None
                    }
                    None => {
                        data_domain(scale_type, &format!("{path}.fallback"))?;
                        None
                    }
                }
            }
        };
        self.fit(spec, scale_type, path)?;
        if let Some(range) = &spec.range {
            match range {
                _ if is_position(channel) => {
                    return Err(fail(
                        format!("{path}.range"),
                        "the plot gives the range of a position channel, which takes only \
                         `reverse`",
                    ))
                }
                scale::Range::Values(value) => {
                    self.values.list(value, &format!("{path}.values"))?;
                }
                scale::Range::Palette(name) => {
                    named(name, &format!("{path}.palette"), "the name of a palette")?
                }
            }
        }
        if let Some(navigation) = &spec.navigation {
            let path = format!("{path}.navigation");
            let continuous = ticked(scale_type) || matches!(scale_type, scale::Type::Custom(_));
            if !(matches!(channel, Channel::X | Channel::Y) && continuous) {
                return Err(fail(
                    &path,
                    "a navigation is valid only on a continuous scale of `x` or `y`",
                ));
            }
            if let Some(navigation_limits::Bounds::Extent(extent)) = &navigation.bounds {
                if !(extent.low.is_finite() && extent.high.is_finite() && extent.low < extent.high)
                {
                    return Err(fail(
                        format!("{path}.extent"),
                        "the bounds of a navigation run from `low` up to `high`",
                    ));
                }
            }
            positive(navigation.min_span, &format!("{path}.min_span"))?;
        }
        Ok(explicit
            .as_ref()
            .map_or(value_type, |array| array.data_type())
            .clone())
    }

    /// Checks an explicit domain and returns its values.
    fn explicit(
        &self,
        value: &proto::ValueRef,
        scale_type: &scale::Type,
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
        if fit == Some(FitKind::Interval) || matches!(scale_type, scale::Type::Threshold(_)) {
            ascends(&array, path)?;
        }
        Ok(array)
    }

    /// Checks the layout of a summary domain against the kind of its scale.
    fn summary(
        &self,
        summary: &proto::ValueRef,
        scale_type: &scale::Type,
        path: &str,
    ) -> Result<()> {
        let summary = self.values.scalar(Some(summary), path)?;
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

    /// Checks the fit fields of `spec`: the renderer fits its domain, they are of one kind, which
    /// its type fits, and their values.
    fn fit(&self, spec: &proto::Scale, scale_type: &scale::Type, path: &str) -> Result<()> {
        let interval = [
            ("zero", spec.zero.is_some()),
            ("nice", spec.nice.is_some()),
            ("pin", spec.pin.is_some()),
            ("domain_empty", spec.domain_empty.is_some()),
        ];
        let interval = interval.iter().find(|(_, set)| *set).map(|(name, _)| *name);
        let sort = spec.sort.is_some().then_some("sort");
        let Some(first) = interval.or(sort) else {
            return Ok(());
        };
        let fitted = match &spec.domain {
            Some(scale::Domain::Explicit(_)) => false,
            Some(scale::Domain::Selection(selection)) => !matches!(
                selection.fallback,
                Some(selection_domain::Fallback::Explicit(_))
            ),
            Some(scale::Domain::Summary(_)) | None => true,
        };
        if !fitted {
            return Err(fail(
                format!("{path}.{first}"),
                format!(
                    "`{first}` is invalid with an explicit domain or an explicit fallback, which \
                     the renderer does not fit"
                ),
            ));
        }
        if let (Some(interval), Some(_)) = (interval, sort) {
            return Err(fail(
                format!("{path}.sort"),
                format!(
                    "`sort` fits categories and `{interval}` an interval, and a scale takes the \
                     fit fields of one kind"
                ),
            ));
        }
        let kind = FitKind::of(scale_type);
        if let Some(field) = interval {
            if kind.is_some_and(|kind| kind != FitKind::Interval) {
                return Err(fail(
                    format!("{path}.{field}"),
                    format!(
                        "`{field}` fits an interval, which a {} scale does not fit",
                        type_name(scale_type)
                    ),
                ));
            }
            if let Some(scale::Nice::TickCount(count)) = spec.nice {
                positive(Some(count), &format!("{path}.tick_count"))?;
            }
            if let Some(empty) = &spec.domain_empty {
                self.values.list(empty, &format!("{path}.domain_empty"))?;
            }
            if let Some(pin) = &spec.pin {
                let value = pin.value.as_ref().expect(REQUIRED);
                let path = format!("{path}.pin.value");
                let pin = self.values.scalar(Some(value), &path)?;
                if !takes_values(domain_values(scale_type), pin.data_type()) {
                    return Err(fail(
                        &path,
                        format!(
                            "a pin of type {} is invalid on a {} scale",
                            pin.data_type(),
                            type_name(scale_type)
                        ),
                    ));
                }
            }
        }
        if let Some(sort) = &spec.sort {
            if kind.is_some_and(|kind| kind != FitKind::Categories) {
                return Err(fail(
                    format!("{path}.sort"),
                    format!(
                        "`sort` fits categories, which a {} scale does not fit",
                        type_name(scale_type)
                    ),
                ));
            }
            if let scale::Sort::Preferred(value) = sort {
                if matches!(scale_type, scale::Type::NestedBand(_)) {
                    return Err(fail(
                        format!("{path}.preferred"),
                        "a preferred order is invalid on a nested band scale",
                    ));
                }
                self.values.list(value, &format!("{path}.preferred"))?;
            }
        }
        Ok(())
    }

    /// Checks an axis: valid on `x` or `y` of a Cartesian plot and on `theta` or `radius` of a polar
    /// plot, without the fields that the other kind of plot draws.
    fn axis(
        &self,
        axis: &proto::Axis,
        channel: Channel,
        kind: CoordKind,
        scale_type: &scale::Type,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        let invalid = |field: &str, reason: &str| Err(fail(format!("{path}.{field}"), reason));
        title(axis.title.as_ref(), &format!("{path}.title"), false)?;
        let polar_orient = matches!(
            axis.label_rotation,
            Some(axis::LabelRotation::LabelOrient(_))
        );
        match (kind, channel) {
            (CoordKind::Cartesian, Channel::X | Channel::Y) => {
                if axis.angle.is_some() {
                    return invalid("angle", "an angle is valid only on the axis of `radius`");
                }
                if axis.route != 0 {
                    return invalid("route", "a route is valid only on the axis of a polar plot");
                }
                if polar_orient {
                    return invalid(
                        "label_orient",
                        "an orient of labels is valid only on the axis of a polar plot",
                    );
                }
            }
            (CoordKind::Polar, Channel::Theta | Channel::Radius) => {
                let cartesian = [
                    ("title", axis.title.is_some()),
                    ("orient", axis.orient != 0),
                    ("label_align", axis.label_align != 0),
                    ("label_baseline", axis.label_baseline != 0),
                    ("levels", !axis.levels.is_empty()),
                ];
                if let Some((field, _)) = cartesian.iter().find(|(_, set)| *set) {
                    return invalid(field, "the axis of a polar plot draws no such part");
                }
                if axis.angle.is_some() && channel != Channel::Radius {
                    return invalid("angle", "an angle is valid only on the axis of `radius`");
                }
            }
            _ => {
                return Err(fail(
                    path,
                    "an axis is valid on `x` or `y` of a Cartesian plot, and on `theta` or \
                     `radius` of a polar plot",
                ))
            }
        }
        classes(&axis.classes, path)?;
        non_negative(axis.tick_size.map(f64::from), &format!("{path}.tick_size"))?;
        non_negative(
            axis.grid_width.map(f64::from),
            &format!("{path}.grid_width"),
        )?;
        self.labels(axis.labels.as_ref(), domain_type, path)?;
        self.tick_placement(
            axis.tick_placement.as_ref().map(Placement::of_axis),
            scale_type,
            domain_type,
            path,
        )?;
        self.levels(&axis.levels, scale_type, domain_type, path)
    }

    /// Checks a legend, on a channel other than a position: a symbol legend or a colorbar of a
    /// continuous color scale, without the fields of the other form.
    fn legend(
        &self,
        legend: &proto::Legend,
        channel: Channel,
        scale_type: &scale::Type,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        classes(&legend.classes, path)?;
        title(legend.title.as_ref(), &format!("{path}.title"), false)?;
        let color = matches!(channel, Channel::Color | Channel::Fill | Channel::Stroke);
        let colorbar = match LegendForm::try_from(legend.r#type).unwrap_or_default() {
            LegendForm::Colorbar => true,
            LegendForm::Symbol => false,
            LegendForm::Unspecified => color && ticked(scale_type),
        };
        let invalid = |field: &str, reason: &str| Err(fail(format!("{path}.{field}"), reason));
        let placement = legend.tick_placement.as_ref().map(Placement::of_legend);
        if colorbar {
            if !color || !ticked(scale_type) {
                return Err(fail(
                    path,
                    "a colorbar is the legend of a linear, log, pow, symlog, or time scale of a \
                     color channel",
                ));
            }
            let symbol = [
                ("wrap", legend.wrap != 0),
                ("columns", legend.columns.is_some()),
                ("label_padding", legend.label_padding.is_some()),
                ("column_gap", legend.column_gap.is_some()),
                ("row_gap", legend.row_gap.is_some()),
                ("swatches", legend.swatches.is_some()),
            ];
            if let Some((field, _)) = symbol.iter().find(|(_, set)| *set) {
                return invalid(field, "a colorbar has no such part");
            }
            non_negative(
                legend.tick_size.map(f64::from),
                &format!("{path}.tick_size"),
            )?;
            self.tick_placement(placement, scale_type, domain_type, path)?;
        } else {
            let colorbar_only = [
                ("thickness", legend.thickness.is_some()),
                ("axis_gap", legend.axis_gap.is_some()),
                ("max_length", legend.max_length.is_some()),
                ("label_align", legend.label_align != 0),
                ("label_baseline", legend.label_baseline != 0),
                ("tick_step", matches!(placement, Some(Placement::Step(_)))),
                ("ticks", legend.ticks.is_some()),
                ("tick_size", legend.tick_size.is_some()),
                ("tick_color", !legend.tick_color.is_empty()),
                ("rule", legend.rule.is_some()),
                ("rule_color", !legend.rule_color.is_empty()),
            ];
            if let Some((field, _)) = colorbar_only.iter().find(|(_, set)| *set) {
                return invalid(field, "a symbol legend has no such part");
            }
            if legend.columns.is_some()
                && proto::LegendWrap::try_from(legend.wrap) == Ok(proto::LegendWrap::Flow)
            {
                return invalid(
                    "columns",
                    "a count of columns lays out a grid, so it is invalid with a flow",
                );
            }
            if !ticked(scale_type) && placement.is_some() {
                return Err(fail(
                    path,
                    "only the legend of a linear, log, pow, symlog, or time scale takes a tick \
                     count or values",
                ));
            }
            self.tick_placement(placement, scale_type, domain_type, path)?;
        }
        self.labels(legend.labels.as_ref(), domain_type, path)
    }

    /// Checks the levels of an axis: valid only on the axis of a nested band scale, with at most
    /// one for each level of its paths, and the pattern of each in the syntax of its values.
    fn levels(
        &self,
        levels: &[proto::NestedAxisLevel],
        scale_type: &scale::Type,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        if levels.is_empty() {
            return Ok(());
        }
        let path = format!("{path}.levels");
        if !matches!(scale_type, scale::Type::NestedBand(_)) {
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
            let Some(labels) = &level.labels else {
                continue;
            };
            let path = format!("{path}[{index}].labels");
            let axis_only = [
                ("markup", !labels.markup.is_empty()),
                ("font_family", !labels.font_family.is_empty()),
                ("font_size", labels.font_size.is_some()),
                ("font_weight", labels.font_weight.is_some()),
                ("color", !labels.color.is_empty()),
            ];
            if let Some((name, _)) = axis_only.iter().find(|(_, set)| *set) {
                return Err(fail(
                    format!("{path}.{name}"),
                    "the labels of every level take the markup and the font of the axis's labels",
                ));
            }
            self.pattern(&labels.format, field.data_type(), &format!("{path}.format"))?;
        }
        Ok(())
    }

    fn labels(
        &self,
        labels: Option<&proto::Labels>,
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

    /// Checks where the ticks of an axis or a colorbar, or the entries of a symbol legend, stand.
    /// On the axis of a band or a point scale, only a list of ticks is valid, and on the axis of a
    /// nested band scale, which ticks at the bands of its leaf, the ticks of the scale.
    fn tick_placement(
        &self,
        placement: Option<Placement>,
        scale_type: &scale::Type,
        domain_type: &DataType,
        path: &str,
    ) -> Result<()> {
        let Some(placement) = placement else {
            return Ok(());
        };
        use scale::Type;
        let at = format!("{path}.{}", placement.name());
        match (scale_type, placement) {
            (Type::NestedBand(_), _) => {
                return Err(fail(
                    at,
                    "the axis of a nested band scale ticks at the bands of its leaf, so its ticks \
                     stand nowhere else",
                ))
            }
            (Type::Band(_) | Type::Point(_), Placement::Count(_) | Placement::Step(_)) => {
                return Err(fail(
                    at,
                    "only a list of ticks is valid on the axis of a band or a point scale",
                ))
            }
            _ => {}
        }
        match placement {
            Placement::Count(count) => positive(Some(count), &at),
            Placement::Values(values) => self.domain_values(values, domain_type, &at),
            Placement::Step(step) => self.tick_step(step, scale_type, &at),
        }
    }

    /// Checks the step of ticks: more than 0, with a unit on a time scale, where it is whole, and
    /// without one on another scale, and a start in the type of the scale's domain.
    fn tick_step(
        &self,
        step: &proto::TickStep,
        scale_type: &scale::Type,
        path: &str,
    ) -> Result<()> {
        positive(Some(step.step), &format!("{path}.step"))?;
        let time = matches!(scale_type, scale::Type::Time(_));
        match (time, step.interval != 0) {
            (true, false) => {
                return Err(fail(
                    format!("{path}.interval"),
                    "the step of the ticks of a time scale has a unit",
                ))
            }
            (false, true) => {
                return Err(fail(
                    format!("{path}.interval"),
                    format!(
                        "a unit of the step of ticks is valid only on a time scale, not on a {} \
                         scale",
                        type_name(scale_type)
                    ),
                ))
            }
            (true, true) if step.step.fract() != 0.0 => {
                return Err(fail(
                    format!("{path}.step"),
                    "the step of the ticks of a time scale is a whole number of its unit",
                ))
            }
            _ => {}
        }
        if let Some(start) = &step.start {
            let path = format!("{path}.start");
            let start = self.values.scalar(Some(start), &path)?;
            if !takes_values(domain_values(scale_type), start.data_type()) {
                return Err(fail(
                    &path,
                    format!(
                        "a start of type {} is invalid on a {} scale",
                        start.data_type(),
                        type_name(scale_type)
                    ),
                ));
            }
        }
        Ok(())
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
fn scale_options(scale_type: &scale::Type, value_type: &DataType, path: &str) -> Result<()> {
    use scale::Type;
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
            let levels = levels(value_type);
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
                &custom.name,
                &format!("{path}.custom.name"),
                "the name of a custom scale",
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

fn data_domain(scale_type: &scale::Type, path: &str) -> Result<()> {
    if matches!(scale_type, scale::Type::Threshold(_)) {
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
        Some(scale::Type::Band(_))
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
    let column_of = |channel: Channel| match &mark.encoding(channel)?.kind {
        Some(encoding::Kind::DirectField(name) | encoding::Kind::ScaledField(name)) => {
            table.column_by_name(name).cloned()
        }
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
        | Channel::X
        | Channel::Y
        | Channel::X2
        | Channel::Y2
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
    (table.column_by_name(name)).ok_or_else(|| {
        fail(
            path,
            format!(
                "the layer's table has no column `{name}`. For a constant, use a `direct_value`"
            ),
        )
    })
}

/// Checks where the mark of a position channel stands in its band: `band_position` from 0 to 1
/// through a band or a nested band scale, and `band_level` below the levels of a nested band
/// scale, each on `x`, `y`, `theta`, or `radius`.
fn band_placement(
    encoding: &proto::Encoding,
    channel: Channel,
    value_type: &DataType,
    path: &str,
) -> Result<()> {
    if encoding.band_position.is_none() && encoding.band_level.is_none() {
        return Ok(());
    }
    let scale_type = (encoding.scale.as_ref()).and_then(|scale| scale.r#type.as_ref());
    let nested = matches!(scale_type, Some(scale::Type::NestedBand(_)));
    let banded = nested || matches!(scale_type, Some(scale::Type::Band(_)));
    let positioned = matches!(
        channel,
        Channel::X | Channel::Y | Channel::Theta | Channel::Radius
    );
    if let Some(position) = encoding.band_position {
        if !positioned || !banded {
            return Err(fail(
                format!("{path}.band_position"),
                "a band position is valid only on `x`, `y`, `theta`, and `radius` through a band \
                 or a nested band scale",
            ));
        }
        within(Some(position), 0.0, 1.0, &format!("{path}.band_position"))?;
    }
    if let Some(level) = encoding.band_level {
        if !positioned || !nested {
            return Err(fail(
                format!("{path}.band_level"),
                "a band level is valid only on `x`, `y`, `theta`, and `radius` through a nested \
                 band scale",
            ));
        }
        let levels = levels(value_type);
        if level as usize >= levels {
            return Err(fail(
                format!("{path}.band_level"),
                format!("the paths have {levels} levels"),
            ));
        }
    }
    Ok(())
}

/// The levels of the paths of a nested band scale: the fields of their struct.
fn levels(value_type: &DataType) -> usize {
    match dictionary_values(value_type) {
        DataType::Struct(fields) => fields.len(),
        _ => 0,
    }
}

/// Whether `value` goes through the channel's scale.
fn is_scaled(value: &encoding::Kind) -> bool {
    matches!(
        value,
        encoding::Kind::ScaledField(_) | encoding::Kind::ScaledValue(_)
    )
}

/// Whether the value of `encoding` or one of its conditional values goes through the channel's
/// scale.
pub(crate) fn scaled(encoding: &proto::Encoding) -> bool {
    encoding.kind.as_ref().is_some_and(is_scaled)
        || (encoding.conditional.iter()).any(|conditional| {
            matches!(
                conditional.kind,
                Some(
                    conditional_value::Kind::ScaledField(_)
                        | conditional_value::Kind::ScaledValue(_)
                )
            )
        })
}

include!(concat!(env!("OUT_DIR"), "/channels.rs"));

/// Checks an encoding of a column of spans: it is on a position channel with a partner, such as
/// `x` or `x2`, of a mark that takes the partner, and the pair names the one column, so the first
/// channel takes the starts and the partner the ends.
fn spans(
    mark: &dyn MarkMessage,
    channel: Channel,
    encoding: &proto::Encoding,
    table: &RecordBatch,
    path: &str,
) -> Result<()> {
    let name = match &encoding.kind {
        Some(encoding::Kind::ScaledField(name) | encoding::Kind::DirectField(name)) => name,
        _ => return Ok(()),
    };
    let spanned =
        (table.column_by_name(name)).is_some_and(|column| span::is_span(column.data_type()));
    if !spanned {
        return Ok(());
    }
    let first = scale_channel(channel);
    let Some(partner) = span::partner(first) else {
        return Err(fail(
            path,
            format!(
                "a column of spans gives a position and its partner, such as `x` and `x2`, and \
                 `{}` has no partner",
                channel.field_name()
            ),
        ));
    };
    if !mark.channels().contains(&partner) {
        return Err(fail(
            path,
            format!(
                "a `{}` mark takes no `{}`, so it takes no column of spans on `{}`",
                mark.name(),
                partner.field_name(),
                first.field_name()
            ),
        ));
    }
    let field = |channel: Channel| {
        mark.encoding(channel)
            .and_then(|encoding| match &encoding.kind {
                Some(encoding::Kind::ScaledField(name) | encoding::Kind::DirectField(name)) => {
                    Some(name.as_str())
                }
                _ => None,
            })
    };
    if field(first) != Some(name) || field(partner) != Some(name) {
        return Err(fail(
            path,
            format!(
                "the column of spans `{name}` gives `{}` and `{}`, so they name it both, or `{}` \
                 is absent",
                first.field_name(),
                partner.field_name(),
                partner.field_name()
            ),
        ));
    }
    Ok(())
}

/// The member of the `tick_placement` of an axis or a legend.
#[derive(Clone, Copy)]
enum Placement<'a> {
    Count(f64),
    Step(&'a proto::TickStep),
    Values(&'a proto::ValueRef),
}

impl<'a> Placement<'a> {
    fn of_axis(placement: &'a axis::TickPlacement) -> Self {
        match placement {
            axis::TickPlacement::TickCount(count) => Placement::Count(*count),
            axis::TickPlacement::TickStep(step) => Placement::Step(step),
            axis::TickPlacement::Values(values) => Placement::Values(values),
        }
    }

    fn of_legend(placement: &'a legend::TickPlacement) -> Self {
        match placement {
            legend::TickPlacement::TickCount(count) => Placement::Count(*count),
            legend::TickPlacement::TickStep(step) => Placement::Step(step),
            legend::TickPlacement::Values(values) => Placement::Values(values),
        }
    }

    /// The name of the member.
    fn name(self) -> &'static str {
        match self {
            Placement::Count(_) => "tick_count",
            Placement::Step(_) => "tick_step",
            Placement::Values(_) => "values",
        }
    }
}
