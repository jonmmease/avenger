use arrow::{
    array::{ArrayRef, RecordBatch},
    datatypes::DataType,
};

use crate::{
    layer::{MarkMessage, MarkType},
    proto::{self, domain, encoding, node::Node, plot::Coord, scale_spec, Channel},
    validate::{dictionary_values, takes_scale},
};

/// Infers the scales of the encodings of `message` from their marks and the types of their values,
/// with the tables and the values that `message` refers to: the absent scale types and domains. It
/// never changes a field that is present, so a producer sets a field to override an inference.
///
/// It infers the scale of an encoding of a column or of a scaled constant:
///
/// - an encoding of `x` or `y` always has a scale, which it gets when it has none
/// - an encoding of another channel gets an inferred scale when it holds a scale whose type is
///   absent, such as `ScaleSpec::default()`, and an encoding without a scale stays one whose
///   values the mark draws as they are
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
/// type, and the reader reports the encoding. Fits, guides, titles, styles, and the kinds of
/// discrete scales stay absent: they are the choice of the producer and the renderer's stylesheets.
pub fn infer_scales(message: &mut proto::Chart, tables: &[RecordBatch], values: &[ArrayRef]) {
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
        if !takes_scale(*channel) {
            continue;
        }
        let data_type = match &encoding.value {
            Some(encoding::Value::Column(name)) => table
                .and_then(|table| table.schema().field_with_name(name).ok().cloned())
                .map(|field| field.data_type().clone()),
            Some(encoding::Value::ScaledConstant(value)) => {
                (values.get(value.index as usize)).map(|value| value.data_type().clone())
            }
            _ => continue,
        };
        if encoding.scale.is_none() && matches!(channel, Channel::X | Channel::Y) {
            encoding.scale = Some(proto::ScaleSpec::default());
        }
        let Some(spec) = &mut encoding.scale else {
            continue;
        };
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
fn scale_type(
    data_type: &DataType,
    channel: Channel,
    mark_type: MarkType,
) -> Option<scale_spec::Type> {
    use scale_spec::Type;
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
