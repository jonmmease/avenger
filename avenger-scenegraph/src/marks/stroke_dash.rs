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
    dashed_geometry(paths, stroke_dash).0
}

pub(crate) fn dashed_geometry<'a>(
    paths: impl IntoIterator<Item = &'a Path>,
    stroke_dash: &[f32],
) -> (Path, Vec<(lyon_path::math::Point, lyon_path::math::Vector)>) {
    let pattern: Vec<_> = stroke_dash
        .iter()
        .copied()
        .filter(|dash| dash.is_finite() && *dash >= 0.0)
        .collect();
    if !pattern.iter().any(|dash| *dash > 0.0) {
        return (combine_paths(paths), Vec::new());
    }
    let mut output = Path::builder();
    let mut caps = Vec::new();
    for path in paths {
        let mut subpath = Path::builder();
        for event in path.iter() {
            subpath.path_event(event);
            if let lyon_path::Event::End { close, .. } = event {
                let path = std::mem::replace(&mut subpath, Path::builder()).build();
                let measurements = PathMeasurements::from_path(&path, 0.05);
                let mut sampler = PathSampler::new(&measurements, &path, &(), SampleType::Distance);
                let length = sampler.length();
                if length == 0.0 {
                    // An explicit zero-length segment starts in the first on-dash.
                    for event in path.iter() {
                        output.path_event(event);
                    }
                    continue;
                }
                let mut ranges: Vec<std::ops::Range<f32>> = Vec::new();
                let mut start = 0.0;
                let mut index = 0;
                while start < length {
                    let amount = pattern[index % pattern.len()];
                    let end = (start + amount).min(length);
                    if index % 2 == 0 {
                        if amount == 0.0 {
                            let sample = sampler.sample(start);
                            let at = sample.position();
                            output.begin(at);
                            output.line_to(at);
                            output.end(false);
                            caps.push((at, sample.tangent()));
                        } else if let Some(previous) = ranges.last_mut().filter(|r| r.end == start)
                        {
                            previous.end = end;
                        } else {
                            ranges.push(start..end);
                        }
                    }
                    start = end;
                    index += 1;
                }
                // Join the on-dash across a closed contour's seam instead of adding two caps.
                if close
                    && ranges.first().is_some_and(|r| r.start == 0.0)
                    && ranges.last().is_some_and(|r| r.end == length)
                {
                    if ranges.len() == 1 {
                        for event in path.iter() {
                            output.path_event(event);
                        }
                        continue;
                    }
                    let first = ranges.remove(0);
                    let last = ranges.pop().unwrap();
                    let mut joined = Path::builder();
                    sampler.split_range(last, &mut joined);
                    sampler.split_range(first, &mut joined);
                    let mut started = false;
                    for event in joined.build().iter() {
                        match event {
                            lyon_path::Event::Begin { .. } if started => {}
                            lyon_path::Event::Begin { .. } => {
                                output.path_event(event);
                                started = true;
                            }
                            lyon_path::Event::End { .. } => {}
                            _ => {
                                output.path_event(event);
                            }
                        }
                    }
                    output.end(false);
                }
                for range in ranges {
                    sampler.split_range(range, &mut output);
                }
            }
        }
    }
    (output.build(), caps)
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

    #[test]
    fn restarts_odd_dash_patterns_at_each_subpath_and_retains_zero_dashes() {
        let mut builder = Path::builder();
        for y in [0., 10.] {
            builder.begin(point(0., y));
            builder.line_to(point(12., y));
            builder.end(false);
        }
        let path = builder.build();
        let dashed = dash_paths([&path], &[2., 1., 3.]);
        let starts: Vec<_> = dashed
            .iter()
            .filter_map(|event| match event {
                Event::Begin { at } => Some(at.to_array()),
                _ => None,
            })
            .collect();
        assert_eq!(
            starts,
            vec![
                [0., 0.],
                [3., 0.],
                [8., 0.],
                [0., 10.],
                [3., 10.],
                [8., 10.]
            ]
        );
        let dotted = dash_paths([&path], &[0., 4.]);
        assert_eq!(crate::path_geometry::zero_length_subpaths(&dotted).len(), 6);
    }

    #[test]
    fn closed_dash_seam_and_zero_gaps_keep_joins() {
        let mut builder = Path::builder();
        builder.begin(point(0., 0.));
        builder.line_to(point(10., 0.));
        builder.line_to(point(10., 10.));
        builder.line_to(point(0., 10.));
        builder.close();
        let path = builder.build();
        let dashed = dash_paths([&path], &[15., 10.]);
        assert_eq!(begin_count(&dashed), 1);
        let joined = dash_paths([&path], &[2., 0.]);
        assert!(matches!(
            joined.iter().last(),
            Some(Event::End { close: true, .. })
        ));
    }

    fn begin_count(path: &Path) -> usize {
        path.iter()
            .filter(|event| matches!(event, Event::Begin { .. }))
            .count()
    }
}
