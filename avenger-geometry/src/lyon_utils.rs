use geo::BooleanOps;
// use geo_booleanop::boolean::BooleanOp;
use geo_types::{Coord, Geometry, LineString, MultiLineString, MultiPolygon, Point, Polygon};
use lyon_path::geom::euclid::{Point2D, UnknownUnit};
use lyon_path::iterator::PathIterator;
use lyon_path::{LineCap, LineJoin, Path, PathEvent};
use lyon_tessellation::{
    geometry_builder::{simple_builder, VertexBuffers},
    StrokeOptions, StrokeTessellator,
};

pub trait IntoGeoType {
    /// Convert the path into a geo-types geometry
    ///
    /// # Arguments
    ///
    /// * `tolerance` - The tolerance to use when flattening curves
    /// * `filled` - If true, treat all paths as filled polygons by forcing closure.
    ///   If false, treat all paths as lines
    fn as_geo_type(&self, tolerance: f32, filled: bool) -> Geometry<f32>;

    /// Convert a trail path with variable width into a geo-types geometry
    ///
    /// # Arguments
    ///
    /// * `tolerance` - The tolerance to use when flattening curves
    /// * `size_attribute_index` - Index of the size attribute in the path's attributes
    fn trail_as_geo_type(&self, tolerance: f32, size_attribute_index: usize) -> Geometry<f32>;
}

impl IntoGeoType for Path {
    fn as_geo_type(&self, tolerance: f32, filled: bool) -> Geometry<f32> {
        let mut current_line: Vec<Coord<f32>> = Vec::new();
        let mut lines: Vec<LineString<f32>> = Vec::new();
        let mut polygons: Vec<Polygon<f32>> = Vec::new();
        let mut current_start: Option<Coord<f32>> = None;

        // Flatten the path and collect coordinates
        for evt in self.iter().flattened(tolerance) {
            match evt {
                PathEvent::Begin { at } => {
                    if !current_line.is_empty() {
                        let line = std::mem::take(&mut current_line);
                        if filled {
                            if line.len() >= 3 {
                                // In filled mode, force close any path with 3 or more points
                                let mut closed_line = line;
                                if closed_line.first() != closed_line.last() {
                                    closed_line.push(closed_line[0]);
                                }
                                polygons.push(Polygon::new(
                                    LineString::new(closed_line),
                                    vec![], // No interior rings
                                ));
                            }
                        } else {
                            lines.push(LineString::new(line));
                        }
                    }
                    let coord = Coord { x: at.x, y: at.y };
                    current_start = Some(coord);
                    current_line.push(coord);
                }
                PathEvent::Line { to, .. } => {
                    current_line.push(Coord { x: to.x, y: to.y });
                }
                PathEvent::End {
                    close: _, first: _, ..
                } => {
                    if !current_line.is_empty() {
                        if filled {
                            if current_line.len() >= 3 {
                                // In filled mode, force close any path with 3 or more points
                                if current_line.first() != current_line.last() {
                                    if let Some(start) = current_start {
                                        current_line.push(start);
                                    }
                                }
                                polygons.push(Polygon::new(
                                    LineString::new(std::mem::take(&mut current_line)),
                                    vec![], // No interior rings
                                ));
                            }
                        } else {
                            lines.push(LineString::new(std::mem::take(&mut current_line)));
                        }
                    }
                    current_start = None;
                }
                _ => {} // We don't need to handle curves as we're using flattened iterator
            }
        }

        // Handle the final path segment if any
        if !current_line.is_empty() {
            if filled {
                if current_line.len() >= 3 && current_line.first() != current_line.last() {
                    if let Some(start) = current_start {
                        current_line.push(start);
                    }
                }
                polygons.push(Polygon::new(LineString::new(current_line), vec![]));
            } else {
                lines.push(LineString::new(current_line));
            }
        }

        // Return the appropriate geometry type based on what we collected
        if filled {
            match (lines.len(), polygons.len()) {
                (0, 0) => {
                    // Empty path - return a point at origin
                    Geometry::Point(Point::new(0.0, 0.0))
                }
                (0, 1) => Geometry::Polygon(polygons.into_iter().next().unwrap()),
                (0, _) => Geometry::MultiPolygon(MultiPolygon(polygons)),
                (_, _) => {
                    // If we have any lines in filled mode, they must be degenerate (less than 3 points)
                    // Convert everything to linestrings for consistency
                    lines.extend(polygons.into_iter().map(|p| p.exterior().clone()));
                    match lines.len() {
                        1 => Geometry::LineString(lines.into_iter().next().unwrap()),
                        _ => Geometry::MultiLineString(MultiLineString(lines)),
                    }
                }
            }
        } else {
            // In unfilled mode, everything becomes a linestring
            match lines.len() {
                0 => {
                    // Empty path - return a point at origin
                    Geometry::Point(Point::new(0.0, 0.0))
                }
                1 => Geometry::LineString(lines.into_iter().next().unwrap()),
                _ => Geometry::MultiLineString(MultiLineString(lines)),
            }
        }
    }

    fn trail_as_geo_type(&self, tolerance: f32, size_attribute_index: usize) -> Geometry<f32> {
        let mut stroke_tessellator = StrokeTessellator::new();
        let mut buffers: VertexBuffers<Point2D<f32, UnknownUnit>, u16> = VertexBuffers::new();

        // Configure stroke options with variable width
        let stroke_options = StrokeOptions::default()
            .with_tolerance(tolerance)
            .with_line_join(LineJoin::Round)
            .with_line_cap(LineCap::Round)
            .with_variable_line_width(size_attribute_index);

        // Tessellate into triangles
        if let Ok(()) = stroke_tessellator.tessellate_path(
            self,
            &stroke_options,
            &mut simple_builder(&mut buffers),
        ) {
            // Convert triangles to polygons
            let mut polygons = Vec::new();
            let vertices = &buffers.vertices;

            // Process triangles in groups of 3 indices
            for triangle in buffers.indices.chunks(3) {
                if triangle.len() == 3 {
                    let coords = vec![
                        Coord {
                            x: vertices[triangle[0] as usize].x,
                            y: vertices[triangle[0] as usize].y,
                        },
                        Coord {
                            x: vertices[triangle[1] as usize].x,
                            y: vertices[triangle[1] as usize].y,
                        },
                        Coord {
                            x: vertices[triangle[2] as usize].x,
                            y: vertices[triangle[2] as usize].y,
                        },
                        // Close the polygon by repeating first point
                        Coord {
                            x: vertices[triangle[0] as usize].x,
                            y: vertices[triangle[0] as usize].y,
                        },
                    ];

                    polygons.push(Polygon::new(LineString::new(coords), vec![]));
                }
            }

            // Union all polygons together
            if let Some(first) = polygons.pop() {
                let result = polygons
                    .into_iter()
                    .fold(MultiPolygon::from(first), |acc, poly| acc.union(&poly));
                match result.0.len() {
                    0 => Geometry::Point(Point::new(0.0, 0.0)),
                    1 => Geometry::Polygon(result.0[0].clone()),
                    _ => Geometry::MultiPolygon(result),
                }
            } else {
                Geometry::Point(Point::new(0.0, 0.0))
            }
        } else {
            // Fallback to point if tessellation fails
            Geometry::Point(Point::new(0.0, 0.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::approx_eq;
    use geo_types::Coord;
    use lyon_path::{math::point, Path};

    fn assert_coords_eq(a: &Coord<f32>, b: &Coord<f32>) {
        assert!(
            approx_eq!(f32, a.x, b.x, ulps = 2),
            "x coordinates differ: {} != {}",
            a.x,
            b.x
        );
        assert!(
            approx_eq!(f32, a.y, b.y, ulps = 2),
            "y coordinates differ: {} != {}",
            a.y,
            b.y
        );
    }

    #[test]
    fn test_simple_line() {
        let mut builder = Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(1.0, 0.0));
        builder.line_to(point(1.0, 1.0));
        builder.end(false);

        let path = builder.build();
        let geometry = path.as_geo_type(0.1, false);

        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 3);
                assert_coords_eq(coords[0], &Coord::<f32> { x: 0.0, y: 0.0 });
                assert_coords_eq(coords[1], &Coord::<f32> { x: 1.0, y: 0.0 });
                assert_coords_eq(coords[2], &Coord::<f32> { x: 1.0, y: 1.0 });
            }
            _ => panic!("Expected LineString"),
        }
    }

    #[test]
    fn test_simple_polygon() {
        let mut builder = Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(1.0, 0.0));
        builder.line_to(point(1.0, 1.0));
        builder.line_to(point(0.0, 1.0));
        builder.end(true);

        let path = builder.build();

        // Test filled mode (should be polygon)
        let geometry = path.clone().as_geo_type(0.1, true);
        match geometry {
            Geometry::Polygon(polygon) => {
                let coords: Vec<_> = polygon.exterior().coords().collect();
                assert_eq!(coords.len(), 5); // 4 unique points + closing point
                assert_coords_eq(coords[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(coords[1], &Coord { x: 1.0, y: 0.0 });
                assert_coords_eq(coords[2], &Coord { x: 1.0, y: 1.0 });
                assert_coords_eq(coords[3], &Coord { x: 0.0, y: 1.0 });
                assert_coords_eq(coords[4], coords[0]); // Should close the polygon
            }
            _ => panic!("Expected Polygon"),
        }

        // Test unfilled mode (should be closed linestring)
        let geometry = path.as_geo_type(0.1, false);
        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 4); // 4 unique points + closing point
                assert_coords_eq(coords[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(coords[1], &Coord { x: 1.0, y: 0.0 });
                assert_coords_eq(coords[2], &Coord { x: 1.0, y: 1.0 });
                assert_coords_eq(coords[3], &Coord { x: 0.0, y: 1.0 });
            }
            _ => panic!("Expected LineString"),
        }
    }

    #[test]
    fn test_multi_line() {
        let mut builder = Path::builder();
        // First line
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(1.0, 0.0));
        builder.end(false);
        // Second line
        builder.begin(point(0.0, 1.0));
        builder.line_to(point(1.0, 1.0));
        builder.end(false);

        let path = builder.build();
        let geometry = path.as_geo_type(0.1, false);

        match geometry {
            Geometry::MultiLineString(multi_line) => {
                assert_eq!(multi_line.0.len(), 2);

                let first_line: Vec<_> = multi_line.0[0].coords().collect();
                assert_eq!(first_line.len(), 2);
                assert_coords_eq(first_line[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(first_line[1], &Coord { x: 1.0, y: 0.0 });

                let second_line: Vec<_> = multi_line.0[1].coords().collect();
                assert_eq!(second_line.len(), 2);
                assert_coords_eq(second_line[0], &Coord { x: 0.0, y: 1.0 });
                assert_coords_eq(second_line[1], &Coord { x: 1.0, y: 1.0 });
            }
            _ => panic!("Expected MultiLineString"),
        }
    }

    #[test]
    fn test_multi_polygon() {
        let mut builder = Path::builder();
        // First square
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(1.0, 0.0));
        builder.line_to(point(1.0, 1.0));
        builder.line_to(point(0.0, 1.0));
        builder.end(true);
        // Second square
        builder.begin(point(2.0, 0.0));
        builder.line_to(point(3.0, 0.0));
        builder.line_to(point(3.0, 1.0));
        builder.line_to(point(2.0, 1.0));
        builder.end(true);

        let path = builder.build();

        // Test filled mode (should be multi-polygon)
        let geometry = path.clone().as_geo_type(0.1, true);
        match geometry {
            Geometry::MultiPolygon(multi_polygon) => {
                assert_eq!(multi_polygon.0.len(), 2);

                // Check first polygon
                let first_poly: Vec<_> = multi_polygon.0[0].exterior().coords().collect();
                assert_eq!(first_poly.len(), 5);
                assert_coords_eq(first_poly[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(first_poly[4], first_poly[0]); // Should close

                // Check second polygon
                let second_poly: Vec<_> = multi_polygon.0[1].exterior().coords().collect();
                assert_eq!(second_poly.len(), 5);
                assert_coords_eq(second_poly[0], &Coord { x: 2.0, y: 0.0 });
                assert_coords_eq(second_poly[4], second_poly[0]); // Should close
            }
            _ => panic!("Expected MultiPolygon"),
        }

        // Test unfilled mode (should be multi-linestring)
        let geometry = path.as_geo_type(0.1, false);
        match geometry {
            Geometry::MultiLineString(multi_line) => {
                assert_eq!(multi_line.0.len(), 2);

                // Check first linestring
                let first_line: Vec<_> = multi_line.0[0].coords().collect();
                assert_eq!(first_line.len(), 4);
                assert_coords_eq(first_line[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(first_line[3], &Coord { x: 0.0, y: 1.0 }); // Should close

                // Check second linestring
                let second_line: Vec<_> = multi_line.0[1].coords().collect();
                assert_eq!(second_line.len(), 4);
                assert_coords_eq(second_line[0], &Coord { x: 2.0, y: 0.0 });
                assert_coords_eq(second_line[3], &Coord { x: 2.0, y: 1.0 }); // Should close
            }
            _ => panic!("Expected MultiLineString"),
        }
    }

    #[test]
    fn test_curved_path() {
        let mut builder = Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.quadratic_bezier_to(point(1.0, 0.0), point(1.0, 1.0));
        builder.end(false);

        let path = builder.build();
        let geometry = path.as_geo_type(0.1, false);

        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert!(
                    coords.len() > 2,
                    "Curve should be flattened into multiple points"
                );
                assert_coords_eq(coords[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(coords.last().unwrap(), &Coord { x: 1.0, y: 1.0 });
            }
            _ => panic!("Expected LineString"),
        }
    }

    #[test]
    fn test_empty_path() {
        let mut builder = Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.end(false);

        let path = builder.build();
        let geometry = path.as_geo_type(0.1, false);

        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 1);
                assert_coords_eq(coords[0], &Coord { x: 0.0, y: 0.0 });
            }
            _ => panic!("Expected LineString"),
        }
    }

    #[test]
    fn test_single_point() {
        let mut builder = Path::builder();
        builder.begin(point(1.0, 2.0));
        builder.end(false);

        let path = builder.build();

        // Test filled mode
        let geometry = path.clone().as_geo_type(0.1, true);
        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 1);
                assert_coords_eq(coords[0], &Coord { x: 1.0, y: 2.0 });
            }
            _ => panic!("Expected LineString"),
        }

        // Test unfilled mode
        let geometry = path.as_geo_type(0.1, false);
        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 1);
                assert_coords_eq(coords[0], &Coord { x: 1.0, y: 2.0 });
            }
            _ => panic!("Expected LineString"),
        }
    }

    #[test]
    fn test_two_points() {
        let mut builder = Path::builder();
        builder.begin(point(1.0, 1.0));
        builder.line_to(point(2.0, 2.0));
        builder.end(false);

        let path = builder.build();

        // Even in filled mode, this should be a LineString as it can't form a polygon
        let geometry = path.clone().as_geo_type(0.1, true);
        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 2);
                assert_coords_eq(coords[0], &Coord { x: 1.0, y: 1.0 });
                assert_coords_eq(coords[1], &Coord { x: 2.0, y: 2.0 });
            }
            _ => panic!("Expected LineString"),
        }
    }

    #[test]
    fn test_almost_closed_triangle() {
        let mut builder = Path::builder();
        builder.begin(point(0.0, 0.0));
        builder.line_to(point(1.0, 0.0));
        builder.line_to(point(0.5, 1.0));
        builder.end(false); // Not explicitly closed

        let path = builder.build();

        // In filled mode, it should become a closed polygon
        let geometry = path.clone().as_geo_type(0.1, true);
        match geometry {
            Geometry::Polygon(polygon) => {
                let coords: Vec<_> = polygon.exterior().coords().collect();
                assert_eq!(coords.len(), 4); // 3 points + closing point
                assert_coords_eq(coords[0], coords[3]); // Should be closed
            }
            _ => panic!("Expected Polygon"),
        }

        // In unfilled mode, it should remain an open linestring
        let geometry = path.as_geo_type(0.1, false);
        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 3); // Should remain open
                assert_coords_eq(coords[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(coords[2], &Coord { x: 0.5, y: 1.0 });
            }
            _ => panic!("Expected LineString"),
        }
    }

    #[test]
    fn test_mixed_degenerate_and_valid() {
        let mut builder = Path::builder();
        // Add a single point
        builder.begin(point(0.0, 0.0));
        builder.end(false);
        // Add a valid triangle
        builder.begin(point(1.0, 1.0));
        builder.line_to(point(2.0, 1.0));
        builder.line_to(point(1.5, 2.0));
        builder.end(true);

        let path = builder.build();

        // In filled mode, we should get a mix of geometries combined into a MultiLineString
        let geometry = path.clone().as_geo_type(0.1, true);
        match geometry {
            Geometry::MultiLineString(multi_line) => {
                assert_eq!(multi_line.0.len(), 2);
                // First should be the single point
                assert_eq!(multi_line.0[0].coords().count(), 1);
                // Second should be the closed triangle
                assert_eq!(multi_line.0[1].coords().count(), 4);
            }
            _ => panic!("Expected MultiLineString"),
        }

        // In unfilled mode, should be the same
        let geometry = path.as_geo_type(0.1, false);
        match geometry {
            Geometry::MultiLineString(multi_line) => {
                assert_eq!(multi_line.0.len(), 2);
                assert_eq!(multi_line.0[0].coords().count(), 1);
                assert_eq!(multi_line.0[1].coords().count(), 3); // Not closed in unfilled mode
            }
            _ => panic!("Expected MultiLineString"),
        }
    }

    #[test]
    fn test_variable_width_trail_single_segment() {
        let mut builder = Path::builder_with_attributes(1);

        // Create a simple path with varying width
        builder.begin(point(0.0, 0.0), &[1.0]);
        builder.line_to(point(1.0, 1.0), &[2.0]);
        builder.line_to(point(2.0, 0.0), &[0.5]);
        builder.line_to(point(3.0, 1.0), &[3.0]);
        builder.end(false);

        let path = builder.build();

        // Convert to geometry with size attribute at index 0
        let geometry = path.trail_as_geo_type(0.1, 0);

        match geometry {
            Geometry::Polygon(p) => {
                assert!(
                    !p.exterior().0.is_empty(),
                    "Polygon exterior should contain at least one point"
                );
            }
            _ => panic!("Expected MultiPolygon geometry"),
        }
    }

    #[test]
    fn test_variable_width_trail_multiple_segments() {
        let mut builder = Path::builder_with_attributes(1);

        // Create a simple path with varying width
        builder.begin(point(0.0, 0.0), &[1.0]);
        builder.line_to(point(1.0, 1.0), &[2.0]);
        builder.line_to(point(2.0, 0.0), &[0.5]);
        builder.end(false);

        builder.begin(point(4.0, 1.0), &[3.0]);
        builder.line_to(point(5.0, 2.0), &[2.0]);
        builder.line_to(point(6.0, 0.0), &[1.0]);
        builder.end(false);

        let path = builder.build();

        // Convert to geometry with size attribute at index 0
        let geometry = path.trail_as_geo_type(0.1, 0);

        match geometry {
            Geometry::MultiPolygon(mp) => {
                assert_eq!(mp.0.len(), 2, "Should contain two polygons");
            }
            _ => panic!("Expected MultiPolygon geometry"),
        }
    }
}
