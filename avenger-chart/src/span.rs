//! Spans: a column of structs of a `start` and an `end` of one numeric or temporal type, which
//! gives a position channel and its partner, such as the bins of a histogram on `x` and `x2`.

use arrow::{
    array::{Array, ArrayRef, AsArray},
    datatypes::DataType,
};

use crate::proto::Channel;

/// The partner of a first position channel: `x2` of `x`, `y2` of `y`, `theta2` of `theta`, and
/// `radius2` of `radius`.
pub fn partner(channel: Channel) -> Option<Channel> {
    match channel {
        Channel::X => Some(Channel::X2),
        Channel::Y => Some(Channel::Y2),
        Channel::Theta => Some(Channel::Theta2),
        Channel::Radius => Some(Channel::Radius2),
        _ => None,
    }
}

/// Whether `channel` is the second channel of a pair, which takes the `end` of a span.
fn second(channel: Channel) -> bool {
    matches!(
        channel,
        Channel::X2 | Channel::Y2 | Channel::Theta2 | Channel::Radius2
    )
}

/// Whether `data_type` is the type of a span: a struct of exactly a `start` and an `end` field, in
/// that order, of one numeric or temporal type.
pub fn is_span(data_type: &DataType) -> bool {
    let DataType::Struct(fields) = data_type else {
        return false;
    };
    match fields.iter().as_slice() {
        [start, end] => {
            start.name() == "start"
                && end.name() == "end"
                && start.data_type() == end.data_type()
                && (start.data_type().is_numeric() || start.data_type().is_temporal())
        }
        _ => false,
    }
}

/// The type of the values that a channel takes from a column of `data_type`: the type of the
/// fields of a span, or `data_type` itself.
pub fn values_type(data_type: &DataType) -> DataType {
    match data_type {
        DataType::Struct(fields) if is_span(data_type) => fields[0].data_type().clone(),
        _ => data_type.clone(),
    }
}

/// The values that `channel` takes from `column`: the `start` of a span for a first channel and
/// its `end` for a second channel, where a null span is a null of both, or `column` itself.
pub fn channel_values(column: &ArrayRef, channel: Channel) -> ArrayRef {
    if !is_span(column.data_type()) {
        return column.clone();
    }
    let spans = column.as_struct();
    let field = spans.column(usize::from(second(channel))).clone();
    match spans.nulls() {
        Some(nulls) if spans.null_count() > 0 => {
            let nulls = arrow::buffer::NullBuffer::union(Some(nulls), field.nulls());
            let data = field.to_data().into_builder().nulls(nulls).build();
            arrow::array::make_array(data.expect("the nulls of a span fit its fields"))
        }
        _ => field,
    }
}
