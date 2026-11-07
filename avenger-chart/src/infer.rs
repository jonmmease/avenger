use arrow::{
    array::{ArrayRef, RecordBatch},
    datatypes::DataType,
};

use crate::{
    layer::{MarkMessage, MarkType},
    proto::{self, conditional_value, domain, encoding, node::Node, plot::Coord, scale, Channel},
    validate::{dictionary_values, scaled, takes_scale},
};

/// Infers the scales of the encodings of `message` from their marks and the types of their values,
/// with the tables and the values that `message` refers to: the absent scale types and domains. It
/// never changes a field that is present, so a producer sets a field to override an inference.
///
/// It infers the scale of each encoding whose value or one of whose conditional values is a
/// `scaled_field` or a `scaled_value`: the scale when the encoding has none, its type when it
/// is absent, and its domain when it is absent. An encoding of plain values has no scale, and the
/// mark draws its values as they are.
///
/// The scale type follows the Arrow type of the values, the channel, and for categories on a
/// position, the mark:
///
/// | Values | Positions | `color`, `fill`, `stroke` | Other numeric channels | `shape`, `pattern`, `stroke_dash` |
/// |---|---|---|---|---|
/// | numbers | linear | linear | linear | ordinal |
/// | dates and timestamps | time | time | time | ordinal |
/// | strings and booleans | band for bars and rects, point otherwise | ordinal | ordinal | ordinal |
/// | structs of paths | nested band | none | none | none |
///
/// A scale with a type whose domain is absent gets the layer's data domain. Values of another type,
/// such as times of day and durations, and a column that the layer's table lacks, get no scale
/// type, and the reader reports the encoding. Fits, axes, legends, titles, styles, and the kinds of
/// discrete scales stay absent: they are the choice of the producer and the renderer's stylesheets.
pub(crate) fn infer_scales(
    message: &mut proto::Chart,
    tables: &[RecordBatch],
    values: &[ArrayRef],
) {
    if let Some(root) = &mut message.root {
        node(root, tables, values);
    }
}

fn node(node: &mut proto::Node, tables: &[RecordBatch], values: &[ArrayRef]) {
    match node.node.as_mut() {
        Some(Node::Plot(plot)) => match plot.coord.as_mut() {
            Some(Coord::Cartesian(cartesian)) => {
                for layer in &mut cartesian.layers {
                    let table = layer
                        .table
                        .and_then(|table| tables.get(table.index as usize));
                    if let Some(mark) = layer.mark.as_mut() {
                        self::mark(mark, table, values);
                    }
                }
            }
            Some(Coord::Polar(polar)) => {
                for layer in &mut polar.layers {
                    let table = layer
                        .table
                        .and_then(|table| tables.get(table.index as usize));
                    if let Some(mark) = layer.mark.as_mut() {
                        self::mark(mark, table, values);
                    }
                }
            }
            Some(Coord::Geo(geo)) => {
                for layer in &mut geo.layers {
                    let table = layer
                        .table
                        .and_then(|table| tables.get(table.index as usize));
                    if let Some(mark) = layer.mark.as_mut() {
                        self::mark(mark, table, values);
                    }
                }
            }
            None => {}
        },
        Some(Node::Concat(proto::Concat { children, .. }))
        | Some(Node::Wrap(proto::Wrap { children, .. })) => {
            for child in children {
                self::node(child, tables, values);
            }
        }
        Some(Node::Empty(_)) | None => {}
    }
}

fn mark(mark: &mut dyn MarkMessage, table: Option<&RecordBatch>, values: &[ArrayRef]) {
    let mark_type = mark.mark_type();
    for channel in mark.channels() {
        let Some(encoding) = mark.encoding_mut(*channel) else {
            continue;
        };
        if !takes_scale(*channel) || !scaled(encoding) {
            continue;
        }
        let column_type = |name: &str| {
            table
                .and_then(|table| table.schema().field_with_name(name).ok().cloned())
                .map(|field| field.data_type().clone())
        };
        let value_type = |value: &proto::ValueRef| {
            (values.get(value.index as usize)).map(|value| value.data_type().clone())
        };
        // The scale takes the type of the encoding's value when it is scaled, and otherwise of the
        // first scaled conditional value.
        let data_type = match &encoding.kind {
            Some(encoding::Kind::ScaledField(name)) => column_type(name),
            Some(encoding::Kind::ScaledValue(value)) => value_type(value),
            _ => {
                let first =
                    (encoding.conditional.iter()).find_map(|conditional| match &conditional.kind {
                        Some(conditional_value::Kind::ScaledField(name)) => Some(column_type(name)),
                        Some(conditional_value::Kind::ScaledValue(value)) => {
                            Some(value_type(value))
                        }
                        _ => None,
                    });
                let Some(data_type) = first else {
                    continue;
                };
                data_type
            }
        };
        let spec = encoding.scale.get_or_insert_with(proto::Scale::default);
        if spec.r#type.is_none() {
            spec.r#type =
                data_type.and_then(|data_type| scale_type(&data_type, *channel, mark_type));
        }
        if spec.r#type.is_some() && spec.domain.is_none() {
            spec.domain = Some(proto::Domain {
                domain: Some(domain::Domain::Data(proto::DataDomain::default())),
            });
        }
    }
}

/// The scale type of values of `data_type` on `channel` of a mark of `mark_type`.
fn scale_type(data_type: &DataType, channel: Channel, mark_type: MarkType) -> Option<scale::Type> {
    use scale::Type;
    enum Values {
        Numbers,
        Times,
        Categories,
        Paths,
    }
    let data_type = dictionary_values(data_type);
    let values = match data_type {
        _ if data_type.is_numeric() => Values::Numbers,
        // A time scale takes dates and timestamps, not times of day, durations, or intervals.
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(..) => Values::Times,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Boolean => {
            Values::Categories
        }
        DataType::Struct(_) => Values::Paths,
        _ => return None,
    };
    let position = matches!(
        channel,
        Channel::X | Channel::Y | Channel::Theta | Channel::Radius
    );
    let categorical = matches!(
        channel,
        Channel::Shape | Channel::Pattern | Channel::StrokeDash
    );
    Some(match values {
        Values::Paths if position => Type::NestedBand(proto::NestedBandScale::default()),
        Values::Paths => return None,
        _ if categorical => Type::Ordinal(proto::OrdinalScale::default()),
        Values::Numbers => Type::Linear(proto::LinearScale::default()),
        Values::Times => Type::Time(proto::TimeScale::default()),
        Values::Categories if position => match mark_type {
            MarkType::Bar | MarkType::Rect => Type::Band(proto::BandScale::default()),
            _ => Type::Point(proto::PointScale::default()),
        },
        Values::Categories => Type::Ordinal(proto::OrdinalScale::default()),
    })
}
