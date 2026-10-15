//! Dashes and stroke outlines, shared by renderers and geometry queries.

use avenger_common::types::{FillRule, StrokeCap, StrokeJoin, LYON_SCENE_MITER_LIMIT};
use lyon_algorithms::measure::{PathMeasurements, PathSampler, SampleType};
use lyon_path::{Path, PathEvent};
use lyon_tessellation::{
    BuffersBuilder, LineCap, LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex,
    TessellationError, VertexBuffers,
};

use crate::render::fill::{contours_to_path, resolve_contours};

/// The path's dashes. Each subpath starts the pattern over.
pub fn dash_path(path: &Path, stroke_dash: &[f32]) -> Path {
    dashed_geometry(path, stroke_dash).0
}

/// The lengths of a dash pattern that dashing uses: the finite, non-negative ones. `None` when
/// none is positive, which draws a solid stroke.
pub fn dash_pattern(stroke_dash: &[f32]) -> Option<Vec<f32>> {
    let pattern: Vec<_> = stroke_dash
        .iter()
        .copied()
        .filter(|dash| dash.is_finite() && *dash >= 0.0)
        .collect();
    pattern.iter().any(|dash| *dash > 0.0).then_some(pattern)
}

/// The path's dashes, and the position and direction of each zero-length dash, which orient its
/// square cap.
pub(crate) fn dashed_geometry(
    path: &Path,
    stroke_dash: &[f32],
) -> (Path, Vec<(lyon_path::math::Point, lyon_path::math::Vector)>) {
    let Some(pattern) = dash_pattern(stroke_dash) else {
        return (path.clone(), Vec::new());
    };
    let mut output = Path::builder();
    let mut caps = Vec::new();
    let mut subpath = Path::builder();
    for event in path.iter() {
        subpath.path_event(event);
        if let lyon_path::Event::End { close, .. } = event {
            let contour = std::mem::replace(&mut subpath, Path::builder()).build();
            let measurements = PathMeasurements::from_path(&contour, 0.05);
            let mut sampler = PathSampler::new(&measurements, &contour, &(), SampleType::Distance);
            let length = sampler.length();
            if length == 0.0 {
                // An explicit zero-length segment starts in the first on-dash.
                for event in contour.iter() {
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
                    } else if let Some(previous) = ranges.last_mut().filter(|r| r.end == start) {
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
                    for event in contour.iter() {
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
    (output.build(), caps)
}

/// Build one nonzero-filled outline of a variable-width round stroke.
pub fn trail_outline(
    centerline: &Path,
    tolerance: f32,
    size_attribute_index: usize,
) -> Result<Path, TessellationError> {
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    StrokeTessellator::new().tessellate_path(
        centerline,
        &StrokeOptions::default()
            .with_tolerance(tolerance)
            .with_line_cap(LineCap::Round)
            .with_line_join(LineJoin::Round)
            .with_variable_line_width(size_attribute_index),
        &mut BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex<'_, '_>| {
            vertex.position().to_array()
        }),
    )?;
    let mut isolated = None;
    let mut has_segment = false;
    for event in centerline.iter_with_attributes() {
        match event {
            lyon_path::Event::Begin {
                at: (at, attributes),
            } => {
                isolated = Some((at, attributes[size_attribute_index]));
                has_segment = false;
            }
            lyon_path::Event::Line { from, to } => {
                has_segment |= from.0 != to.0;
                if let Some((_, size)) = &mut isolated {
                    *size = size.max(to.1[size_attribute_index]);
                }
            }
            lyon_path::Event::End { .. } => {
                if let Some((at, size)) = isolated.take().filter(|_| !has_segment) {
                    if size > 0.0 {
                        lyon_tessellation::FillTessellator::new().tessellate_circle(
                            at,
                            size / 2.0,
                            &lyon_tessellation::FillOptions::default().with_tolerance(tolerance),
                            &mut BuffersBuilder::new(
                                &mut buffers,
                                |vertex: lyon_tessellation::FillVertex<'_>| {
                                    vertex.position().to_array()
                                },
                            ),
                        )?;
                    }
                }
            }
            _ => has_segment = true,
        }
    }
    Ok(stroke_triangles_to_path(&buffers))
}

/// Explicit zero-length subpaths have cap geometry. Bare move commands do not.
pub fn zero_length_subpaths(path: &Path) -> Vec<lyon_path::math::Point> {
    let mut result = Vec::new();
    let mut segment = false;
    let mut extent = false;
    for event in path.iter() {
        match event {
            PathEvent::Begin { .. } => {
                segment = false;
                extent = false;
            }
            PathEvent::Line { from, to } => {
                segment = true;
                extent |= from != to;
            }
            PathEvent::Quadratic { from, ctrl, to } => {
                segment = true;
                extent |= from != to || from != ctrl;
            }
            PathEvent::Cubic {
                from,
                ctrl1,
                ctrl2,
                to,
            } => {
                segment = true;
                extent |= from != to || from != ctrl1 || from != ctrl2;
            }
            PathEvent::End { first, .. } if segment && !extent => result.push(first),
            _ => {}
        }
    }
    result
}

/// Whether a stroke draws as its filled outline because native strokes draw some of its caps
/// differently: butt caps on zero-length dashes or subpaths, which PDFium inks, square caps there,
/// which native strokes align with the axes, and the round caps of a dashed stroke's zero-length
/// subpaths, which resvg omits.
pub fn needs_explicit_caps(path: &Path, dash: Option<&[f32]>, cap: StrokeCap) -> bool {
    let pattern = dash.and_then(dash_pattern);
    let zero_length_subpath = !zero_length_subpaths(path).is_empty();
    if cap == StrokeCap::Round {
        return pattern.is_some() && zero_length_subpath;
    }
    let zero_length_dash = pattern.is_some_and(|pattern| {
        // An odd-length pattern repeats with its dash and gap lengths swapped.
        let odd = pattern.len() % 2 == 1;
        pattern
            .iter()
            .enumerate()
            .any(|(index, length)| *length == 0.0 && (odd || index % 2 == 0))
    });
    zero_length_dash || zero_length_subpath
}

/// The options that tessellate a scene stroke.
pub fn stroke_options(width: f32, cap: StrokeCap, join: StrokeJoin) -> StrokeOptions {
    StrokeOptions::default()
        .with_tolerance(0.05)
        .with_miter_limit(LYON_SCENE_MITER_LIMIT)
        .with_line_width(width)
        .with_line_cap(cap.into())
        .with_line_join(join.into())
}

/// The stroke of `centerline` as one filled region, which renderers fill with the stroke's paint
/// in place of a native stroke. It holds the stroke's dashes and a cap shape at each zero-length
/// dash and subpath, merged under the nonzero rule so that translucent paint applies once where
/// they overlap. Square caps follow the path's direction at zero-length dashes, and align with the
/// x-axis at zero-length subpaths.
pub fn stroke_outline(
    centerline: &Path,
    dash: Option<&[f32]>,
    width: f32,
    cap: StrokeCap,
    join: StrokeJoin,
) -> Result<Path, TessellationError> {
    if width <= 0.0 {
        return Ok(Path::new());
    }
    let (path, mut caps) = match dash {
        Some(dash) => dashed_geometry(centerline, dash),
        None => (centerline.clone(), Vec::new()),
    };
    caps.extend(
        zero_length_subpaths(centerline)
            .into_iter()
            .map(|at| (at, lyon_path::math::vector(1.0, 0.0))),
    );
    let mut segments = Path::builder();
    let mut subpath = Path::builder();
    for event in path.iter() {
        subpath.path_event(event);
        if matches!(event, PathEvent::End { .. }) {
            let subpath = std::mem::replace(&mut subpath, Path::builder()).build();
            if zero_length_subpaths(&subpath).is_empty() {
                for event in subpath.iter() {
                    segments.path_event(event);
                }
            }
        }
    }
    let mut buffers = VertexBuffers::new();
    StrokeTessellator::new().tessellate_path(
        &segments.build(),
        &stroke_options(width, cap, join),
        &mut BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex<'_, '_>| {
            vertex.position().to_array()
        }),
    )?;
    let mut fill = lyon_tessellation::FillTessellator::new();
    for (at, tangent) in caps {
        let mut builder =
            BuffersBuilder::new(&mut buffers, |vertex: lyon_tessellation::FillVertex<'_>| {
                vertex.position().to_array()
            });
        let options = lyon_tessellation::FillOptions::default().with_tolerance(0.05);
        match cap {
            StrokeCap::Butt => {}
            StrokeCap::Round => {
                fill.tessellate_circle(at, width / 2.0, &options, &mut builder)?;
            }
            StrokeCap::Square => {
                let along = tangent.normalize() * (width / 2.0);
                let across = lyon_path::math::vector(-along.y, along.x);
                let mut square = Path::builder();
                square.begin(at - along - across);
                square.line_to(at + along - across);
                square.line_to(at + along + across);
                square.line_to(at - along + across);
                square.close();
                fill.tessellate_path(&square.build(), &options, &mut builder)?;
            }
        }
    }
    Ok(stroke_triangles_to_path(&buffers))
}

fn stroke_triangles_to_path(buffers: &VertexBuffers<[f32; 2], u32>) -> Path {
    let triangles: Vec<_> = buffers
        .indices
        .chunks_exact(3)
        .filter_map(|indices| {
            let [a, mut b, mut c] = [
                buffers.vertices[indices[0] as usize],
                buffers.vertices[indices[1] as usize],
                buffers.vertices[indices[2] as usize],
            ];
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area == 0.0 {
                return None;
            }
            // Consistent winding makes overlapping stroke triangles contribute union coverage.
            if area < 0.0 {
                std::mem::swap(&mut b, &mut c);
            }
            Some(vec![a, b, c])
        })
        .collect();
    let shapes = resolve_contours(&triangles, FillRule::NonZero);
    contours_to_path(&shapes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyon_path::{geom::point, Event, Path};

    #[test]
    fn non_positive_dash_pattern_keeps_solid_path() {
        let mut builder = Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(10.0, 0.0));
        builder.end(false);
        let path = builder.build();

        let dashed = dash_path(&path, &[0.0]);

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
            let dashed = dash_path(&path, &pattern);
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
        let dashed = dash_path(&path, &[2., 1., 3.]);
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
        let dotted = dash_path(&path, &[0., 4.]);
        assert_eq!(zero_length_subpaths(&dotted).len(), 6);
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
        let dashed = dash_path(&path, &[15., 10.]);
        assert_eq!(begin_count(&dashed), 1);
        let joined = dash_path(&path, &[2., 0.]);
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
