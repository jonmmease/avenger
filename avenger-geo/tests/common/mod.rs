//! Helpers shared by the integration tests.

use avenger_geo::stream::StreamEvent;
use avenger_geo::{Projector, RecordingSink};
use geo_traits::GeometryTrait;

/// The bounds `[[x0, y0], [x1, y1]]` of the finite points that `geometry`
/// projects to, or `None` when there are none.
pub fn bounds(
    projector: &Projector,
    geometry: &impl GeometryTrait<T = f64>,
) -> Option<[[f64; 2]; 2]> {
    let mut sink = RecordingSink::default();
    projector.project_to_sink(geometry, &mut sink);
    event_bounds(&sink.events)
}

/// The bounds of the finite points among `events`.
pub fn event_bounds(events: &[StreamEvent]) -> Option<[[f64; 2]; 2]> {
    events
        .iter()
        .filter_map(|event| match *event {
            StreamEvent::Point(x, y) if x.is_finite() && y.is_finite() => Some([x, y]),
            _ => None,
        })
        .fold(None, |bounds, [x, y]| {
            let [[x0, y0], [x1, y1]] = bounds.unwrap_or([[x, y], [x, y]]);
            Some([[x0.min(x), y0.min(y)], [x1.max(x), y1.max(y)]])
        })
}
