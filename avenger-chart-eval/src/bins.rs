//! Bars of bins: bars that span their continuous key to the key's partner, as a histogram of
//! pre-binned data does, whose key axis ticks at the edges of the bins.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array},
    compute::cast,
    datatypes::{DataType, Float64Type},
};
use avenger_chart::{
    layer::{LayerRef, MarkType},
    proto::{cartesian_layer::Mark, encoding, scale::Type, Channel, Orient},
    resolve::{Layer, Resolved},
    span, FitKind,
};

/// How near two edges, as a fraction of the bin width, count as one.
const EPSILON: f64 = 1e-9;

/// The key channel of a bar or an area of `layer`, along which its marks stand at their values,
/// and `x` for a line: `y` for a horizontal bar or area, and `x` otherwise. `None` for another
/// mark. `continuous` says whether the layer's channel goes through a continuous scale.
pub(crate) fn key_channel(layer: &Layer, continuous: impl Fn(Channel) -> bool) -> Option<Channel> {
    let LayerRef::Cartesian(cartesian) = layer.layer else {
        return None;
    };
    let orient = match cartesian.mark.as_ref()? {
        Mark::Bar(bar) => bar.orient,
        Mark::Area(area) => area.orient,
        Mark::Line(_) => Orient::Vertical as i32,
        _ => return None,
    };
    Some(match Orient::try_from(orient).unwrap_or_default() {
        Orient::Horizontal => Channel::Y,
        Orient::Vertical => Channel::X,
        Orient::Unspecified if continuous(Channel::X) && !continuous(Channel::Y) => Channel::Y,
        Orient::Unspecified => Channel::X,
    })
}

/// The edges of the bins of the bars of the scale group `index`, ascending, when the group is the
/// continuous numeric key of bars that span their bins: the lattice of uniform bins from the least
/// start to the greatest end, so the edges of empty bins keep their ticks, or else the distinct
/// starts and ends. The edges of several bar layers on the scale join, and other layers give none.
/// `None` for another group, and for a bar spanned on its value axis, as a Gantt chart's tasks
/// are.
pub(crate) fn edges(resolved: &Resolved, index: usize, kinds: &[FitKind]) -> Option<ArrayRef> {
    let group = &resolved.scales[index];
    let partner = span::partner(group.channel)?;
    let numeric = matches!(
        group.spec.r#type,
        Some(Type::Linear(_) | Type::Log(_) | Type::Pow(_) | Type::Symlog(_))
    );
    if !numeric || kinds.get(index) != Some(&FitKind::Interval) {
        return None;
    }
    let mut bins: Vec<(f64, f64)> = Vec::new();
    for member in (group.members.iter()).filter(|member| member.channel == group.channel) {
        let layer = &resolved.tree.layers[member.layer];
        if layer.mark.mark_type() != MarkType::Bar || layer.mark.encoding(partner).is_none() {
            continue;
        }
        let continuous = |channel: Channel| {
            (resolved.scales.iter().zip(kinds)).any(|(group, kind)| {
                *kind == FitKind::Interval
                    && (group.members.iter())
                        .any(|other| other.layer == member.layer && other.channel == channel)
            })
        };
        if key_channel(layer, continuous) != Some(group.channel) {
            continue;
        }
        let (Some(starts), Some(ends)) = (values(layer, group.channel), values(layer, partner))
        else {
            continue;
        };
        for row in 0..starts.len() {
            if starts.is_valid(row) && ends.is_valid(row) {
                let (start, end) = (starts.value(row), ends.value(row));
                if start.is_finite() && end.is_finite() && start != end {
                    bins.push((start.min(end), start.max(end)));
                }
            }
        }
    }
    if bins.is_empty() {
        return None;
    }
    Some(Arc::new(Float64Array::from(
        lattice(&bins).unwrap_or_else(|| distinct(&bins)),
    )))
}

/// The values of the field of `channel` of `layer` as numbers, from its column, or the starts or
/// the ends of a column of spans.
fn values(layer: &Layer, channel: Channel) -> Option<Float64Array> {
    let name = match &layer.mark.encoding(channel)?.kind {
        Some(encoding::Kind::ScaledField(name) | encoding::Kind::DirectField(name)) => name,
        _ => return None,
    };
    let column = span::channel_values(layer.table.column_by_name(name)?, channel);
    let numbers = cast(&column, &DataType::Float64).ok()?;
    Some(numbers.as_primitive::<Float64Type>().clone())
}

/// The lattice of uniform bins, from the least start to the greatest end, when every bin has one
/// width and every start lies on one lattice of that width.
fn lattice(bins: &[(f64, f64)]) -> Option<Vec<f64>> {
    let width = bins[0].1 - bins[0].0;
    let low = bins.iter().map(|bin| bin.0).fold(f64::INFINITY, f64::min);
    let high = bins
        .iter()
        .map(|bin| bin.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let near = |a: f64, b: f64| (a - b).abs() <= EPSILON * width.abs().max(1.0);
    let uniform = bins.iter().all(|(start, end)| {
        let steps = (start - low) / width;
        near(end - start, width) && near(steps, steps.round())
    });
    if !uniform {
        return None;
    }
    let count = ((high - low) / width).round() as usize;
    Some((0..=count).map(|step| low + step as f64 * width).collect())
}

/// The distinct starts and ends of `bins`, ascending.
fn distinct(bins: &[(f64, f64)]) -> Vec<f64> {
    let mut edges: Vec<f64> = bins
        .iter()
        .flat_map(|(start, end)| [*start, *end])
        .collect();
    edges.sort_by(f64::total_cmp);
    edges.dedup_by(|a, b| (*a - *b).abs() <= EPSILON * a.abs().max(1.0));
    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_bins_give_their_lattice_and_uneven_bins_their_edges() {
        // An empty bin from 10 to 15 keeps its edges.
        let bins = [(0.0, 5.0), (5.0, 10.0), (15.0, 20.0)];
        assert_eq!(lattice(&bins), Some(vec![0.0, 5.0, 10.0, 15.0, 20.0]));
        let uneven = [(0.0, 1.0), (1.0, 3.0), (3.0, 10.0)];
        assert_eq!(lattice(&uneven), None);
        assert_eq!(distinct(&uneven), vec![0.0, 1.0, 3.0, 10.0]);
        // Bins off the lattice of the first are uneven.
        let shifted = [(0.0, 5.0), (7.0, 12.0)];
        assert_eq!(lattice(&shifted), None);
    }
}
