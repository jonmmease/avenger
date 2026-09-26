use lyon_algorithms::measure::{PathMeasurements, PathSampler, SampleType};
use lyon_path::Path;

pub(crate) fn combine_paths<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Path {
    let mut builder = Path::builder();
    for path in paths {
        for event in path.iter() {
            builder.path_event(event);
        }
    }
    builder.build()
}

pub fn dash_paths<'a>(paths: impl IntoIterator<Item = &'a Path>, stroke_dash: &[f32]) -> Path {
    let dash_pattern = stroke_dash
        .iter()
        .copied()
        .filter(|dash| dash.is_finite() && *dash >= 0.0)
        .collect::<Vec<_>>();
    if !dash_pattern.iter().any(|dash| *dash > 0.0) {
        return combine_paths(paths);
    }

    let mut dash_path_builder = Path::builder();

    for path in paths {
        let path_measurements = PathMeasurements::from_path(path, 0.1);
        let mut sampler = PathSampler::new(&path_measurements, path, &(), SampleType::Distance);
        let line_len = sampler.length();
        let mut dash_idx = 0usize;
        let mut start_dash_dist = 0.0f32;
        let mut draw = true;

        while start_dash_dist < line_len {
            let dash_len = dash_pattern[dash_idx];
            let end_dash_dist = (start_dash_dist + dash_len).min(line_len);

            if draw && dash_len > 0.0 {
                sampler.split_range(start_dash_dist..end_dash_dist, &mut dash_path_builder);
            }

            start_dash_dist = end_dash_dist;
            dash_idx = (dash_idx + 1) % dash_pattern.len();
            draw = !draw;
        }
    }

    dash_path_builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyon_path::{geom::point, Event, Path};

    #[test]
    fn dashes_path_into_multiple_drawn_segments() {
        let mut builder = Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(10.0, 0.0));
        builder.end(false);
        let path = builder.build();

        let dashed = dash_paths(std::iter::once(&path), &[2.0, 2.0]);

        assert_eq!(begin_count(&dashed), 3);
    }

    #[test]
    fn non_positive_dash_pattern_keeps_solid_path() {
        let mut builder = Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(10.0, 0.0));
        builder.end(false);
        let path = builder.build();

        let dashed = dash_paths(std::iter::once(&path), &[0.0]);

        assert_eq!(begin_count(&dashed), 1);
        assert_eq!(dashed.iter().count(), path.iter().count());
    }

    #[test]
    fn zero_length_gaps_preserve_drawn_length() {
        let mut builder = Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(10.0, 0.0));
        builder.end(false);
        let path = builder.build();
        for (pattern, expected) in [([1.0, 0.0], 10.0), ([0.0, 1.0], 0.0), ([1.0, 1.0], 5.0)] {
            let dashed = dash_paths(std::iter::once(&path), &pattern);
            let length: f32 = dashed
                .iter()
                .filter_map(|event| match event {
                    Event::Line { from, to } => Some(from.distance_to(to)),
                    _ => None,
                })
                .sum();
            assert!((length - expected).abs() < 1e-4, "{pattern:?}: {length}");
        }
    }

    fn begin_count(path: &Path) -> usize {
        path.iter()
            .filter(|event| matches!(event, Event::Begin { .. }))
            .count()
    }
}
