//! A reader's changes of a chart in the format: the clauses of avenger-selection and the identities
//! of scales as a [`ChartChange`](avenger_chart::proto::ChartChange) holds them, for a host that
//! sends the changes to the host that builds the chart, such as a page to its server.

use avenger_chart::{
    builder::Value,
    proto::{self, plot_key, scale_key},
    resolve::{Id, Identity},
    ChangeMessage,
};
use avenger_selection::Clause;

use crate::{convert::selection, EvalError, ScaleIdentity};

/// The clause of the format of `clause`, with the values of its conditions and its partition. A
/// range or a polygon goes without its cells, since a box snaps its ends to the cells that it
/// covers, and a lasso writes none.
pub fn clause_to_format(clause: &Clause) -> Result<proto::ClauseBuilder, EvalError> {
    selection::format_clause(clause)
}

/// The clause of avenger-selection of `clause`, a clause of `change`, whose values its references
/// read.
pub fn clause_from_format(
    change: &ChangeMessage,
    clause: &proto::Clause,
) -> Result<Clause, EvalError> {
    selection::clause(change, clause)
}

/// The key of the format of the scale `identity`.
pub fn scale_key(identity: &ScaleIdentity) -> proto::ScaleKeyBuilder {
    match identity {
        ScaleIdentity::Family { family, channel } => {
            proto::ScaleKey::family(*channel, family.clone())
        }
        ScaleIdentity::Plot {
            plot,
            channel,
            index,
        } => proto::ScaleKey::plot(*channel, plot_key(plot)).position(*index as u32),
    }
}

fn plot_key(identity: &Identity) -> proto::PlotKeyBuilder {
    let key = match &identity.id {
        Id::Given(id) => proto::PlotKey::given(id.clone()),
        Id::Path(path) => proto::PlotKey::path(path.clone()),
    };
    key.partition((identity.partition.iter()).map(|(column, value)| {
        proto::PartitionValue::new(column.clone(), Value::scalar(value.clone()))
    }))
}

/// The scale identity of `key`, a key of `change`, whose values the partition of its plot reads.
pub fn scale_identity(change: &ChangeMessage, key: &proto::ScaleKey) -> ScaleIdentity {
    let channel = key.channel();
    match key.owner.as_ref().expect("a valid scale key has an owner") {
        scale_key::Owner::Family(family) => ScaleIdentity::Family {
            family: family.clone(),
            channel,
        },
        scale_key::Owner::Plot(plot) => ScaleIdentity::Plot {
            plot: Identity {
                id: match plot.id.as_ref().expect("a valid plot key has an id") {
                    plot_key::Id::Given(id) => Id::Given(id.clone()),
                    plot_key::Id::Path(path) => Id::Path(path.clone()),
                },
                partition: (plot.partition.iter())
                    .map(|value| {
                        let reference = value.value.as_ref().expect("a valid partition value");
                        (value.column.clone(), change.value(reference).clone())
                    })
                    .collect(),
            },
            channel,
            index: key.position as usize,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, Float64Array, StringArray};
    use avenger_chart::proto::Channel;
    use avenger_scales::scalar::Scalar;
    use avenger_selection::Condition;

    use super::*;

    fn strings(values: &[&str]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    fn numbers(values: &[Option<f64>]) -> ArrayRef {
        Arc::new(Float64Array::from(values.to_vec()))
    }

    /// `change` after a trip through its bytes.
    fn through_bytes(change: proto::ChartChangeBuilder) -> ChangeMessage {
        let bytes = ChangeMessage::new(change).unwrap().to_bytes().unwrap();
        ChangeMessage::from_bytes(bytes).unwrap()
    }

    #[test]
    fn a_clause_of_a_widget_with_each_kind_of_condition_round_trips_through_the_format() {
        let set = Condition::set(
            vec!["origin".to_string()],
            vec![Some("color".to_string())],
            vec![strings(&["USA", "Japan"])],
        )
        .unwrap()
        .complement()
        .unwrap();
        let clause = Clause::new(Identity::owner_name("origins", "plot"))
            .partition("cylinders", Scalar::new(strings(&["4"])))
            .conditions([
                set,
                Condition::Range {
                    column: "weight".to_string(),
                    channel: Some("x".to_string()),
                    ends: numbers(&[Some(2000.0), None]),
                    exclude_end: true,
                    cells: None,
                },
                Condition::Polygon {
                    x_column: "weight".to_string(),
                    y_column: "mpg".to_string(),
                    polygons: vec![(
                        numbers(&[Some(0.0), Some(1.0), Some(1.0)]),
                        numbers(&[Some(0.0), Some(0.0), Some(1.0)]),
                    )],
                    cells: None,
                },
                Condition::CyclicRange {
                    column: "longitude".to_string(),
                    channel: None,
                    start: 170.0,
                    end: -170.0,
                    period: 360.0,
                },
            ]);
        let format = clause_to_format(&clause).unwrap();
        let message = through_bytes(
            proto::ChartChange::builder()
                .selections([proto::SelectionClauses::new("brush").clauses([format])]),
        );
        let read = &message.message().selections[0].clauses[0];
        assert_eq!(
            (read.widget.as_str(), read.owner.as_str()),
            ("origins", "plot")
        );
        let back = clause_from_format(&message, read).unwrap();
        assert_eq!(format!("{back:?}"), format!("{clause:?}"));
    }

    #[test]
    fn scale_identities_round_trip_through_the_format() {
        let identities = [
            ScaleIdentity::Family {
                family: "weight".to_string(),
                channel: Channel::X,
            },
            ScaleIdentity::Plot {
                plot: Identity {
                    id: Id::Path("root.concat.children[*].plot".to_string()),
                    partition: [("origin".to_string(), strings(&["USA"]))].into(),
                },
                channel: Channel::Y,
                index: 1,
            },
        ];
        let domains = (identities.iter()).map(|identity| {
            proto::ScaleDomain::new(scale_key(identity), Value::list(numbers(&[Some(0.0)])))
        });
        let message = through_bytes(proto::ChartChange::builder().domains(domains));
        let read: Vec<ScaleIdentity> = (message.message().domains.iter())
            .map(|domain| scale_identity(&message, domain.scale.as_ref().unwrap()))
            .collect();
        assert_eq!(read, identities);
    }
}
