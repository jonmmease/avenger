//! The canonical form of a chart, into which a reader rewrites every chart that it reads, so two
//! spellings of one chart make equal charts and equal bytes.

use arrow::{array::ArrayRef, datatypes::DataType};

use crate::{proto, walk::VisitValues, Chart};

/// Rewrites `chart`, which is valid, into the canonical form.
pub(crate) fn normalize(chart: &mut Chart) {
    chart.values = canonical_values(&mut chart.message, &chart.values);
}

/// The values that the fields of `message` refer to, in the order that `Chart.values` describes,
/// with the fields renumbered: one value for each field, the values of each type together, the
/// types and the values of each type in the order of the first field that refers to them.
fn canonical_values(message: &mut proto::Chart, values: &[ArrayRef]) -> Vec<ArrayRef> {
    let mut references: Vec<usize> = Vec::new();
    message.visit_values(&mut |value| references.push(value.index as usize));
    let mut types: Vec<&DataType> = Vec::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (position, index) in references.iter().enumerate() {
        let data_type = values[*index].data_type();
        let group = match types.iter().position(|known| *known == data_type) {
            Some(group) => group,
            None => {
                types.push(data_type);
                groups.push(Vec::new());
                groups.len() - 1
            }
        };
        groups[group].push(position);
    }
    let mut renumbered = vec![0; references.len()];
    let mut ordered = Vec::with_capacity(references.len());
    for position in groups.into_iter().flatten() {
        renumbered[position] = ordered.len() as u32;
        ordered.push(values[references[position]].clone());
    }
    let mut position = 0;
    message.visit_values_mut(&mut |value| {
        value.index = renumbered[position];
        position += 1;
    });
    ordered
}
