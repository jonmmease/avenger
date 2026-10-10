use geo_types::{Coord, Geometry, LineString, MultiLineString, MultiPolygon, Point, Polygon};
use lyon_path::{iterator::PathIterator, Path, PathEvent};

pub trait IntoGeoType {
    /// Convert the path into a geo-types geometry
    ///
    /// # Arguments
    ///
    /// * `tolerance` - The tolerance to use when flattening curves
    /// * `filled` - If true, treat all paths as filled polygons by forcing closure.
    ///   If false, treat all paths as lines
    fn as_geo_type(&self, tolerance: f32, filled: bool) -> Geometry<f32>;

    /// Resolve filled polygons using the selected winding rule.
    fn filled_geo_type(
        &self,
        tolerance: f32,
        rule: avenger_common::types::FillRule,
    ) -> Geometry<f32>;

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
        if filled {
            return self.filled_geo_type(tolerance, avenger_common::types::FillRule::NonZero);
        }
        let mut current_line = Vec::new();
        let mut lines = Vec::new();
        for event in self.iter().flattened(tolerance) {
            match event {
                PathEvent::Begin { at } => current_line.push(Coord { x: at.x, y: at.y }),
                PathEvent::Line { to, .. } => current_line.push(Coord { x: to.x, y: to.y }),
                PathEvent::End { .. } => {
                    lines.push(LineString::new(std::mem::take(&mut current_line)))
                }
                _ => unreachable!("flattened paths contain only line segments"),
            }
        }
        match lines.len() {
            0 => Geometry::Point(Point::new(0.0, 0.0)),
            1 => Geometry::LineString(lines.pop().unwrap()),
            _ => Geometry::MultiLineString(MultiLineString(lines)),
        }
    }

    fn filled_geo_type(
        &self,
        tolerance: f32,
        rule: avenger_common::types::FillRule,
    ) -> Geometry<f32> {
        let shapes = avenger_scenegraph::path_geometry::filled_contours(self, tolerance, rule);
        let polygons: Vec<_> = shapes
            .into_iter()
            .map(|shape| {
                let mut rings = shape.into_iter().map(LineString::from);
                Polygon::new(
                    rings.next().expect("resolved polygon has an exterior"),
                    rings.collect(),
                )
            })
            .collect();
        // Retain isolated point/line subpaths for stroked-path picking.
        let lines = match self.as_geo_type(tolerance, false) {
            Geometry::LineString(line) => vec![line],
            Geometry::MultiLineString(lines) => lines.0,
            _ => Vec::new(),
        };
        let mut degenerate: Vec<_> = lines
            .into_iter()
            .filter(|line| line.0.len() < 3)
            .map(Geometry::LineString)
            .collect();
        if !degenerate.is_empty() {
            degenerate.extend(polygons.into_iter().map(Geometry::Polygon));
            return if degenerate.len() == 1 {
                degenerate.pop().unwrap()
            } else {
                Geometry::GeometryCollection(geo_types::GeometryCollection(degenerate))
            };
        }
        if polygons.len() == 1 {
            Geometry::Polygon(polygons.into_iter().next().unwrap())
        } else {
            Geometry::MultiPolygon(MultiPolygon(polygons))
        }
    }

    fn trail_as_geo_type(&self, tolerance: f32, size_attribute_index: usize) -> Geometry<f32> {
        match avenger_scenegraph::path_geometry::trail_outline(
            self,
            tolerance,
            size_attribute_index,
        ) {
            Ok(outline) => {
                outline.filled_geo_type(tolerance, avenger_common::types::FillRule::NonZero)
            }
            Err(_) => Geometry::MultiPolygon(MultiPolygon::new(Vec::new())),
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

        let geometry = path.clone().as_geo_type(0.1, true);
        assert!(geo::Contains::contains(&geometry, &Point::new(0.5, 0.5)));
        assert!(!geo::Contains::contains(&geometry, &Point::new(1.5, 0.5)));

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

        let geometry = path.clone().as_geo_type(0.1, true);
        assert!(geo::Contains::contains(&geometry, &Point::new(1.5, 1.4)));
        let Geometry::GeometryCollection(parts) = geometry else {
            panic!("expected filled area and isolated point")
        };
        assert_eq!(parts.0.len(), 2);

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

        use geo::Intersects;
        for (x, y) in [(0.0, 0.0), (3.0, 1.0), (4.3, 1.0)] {
            assert!(geometry.intersects(&Point::new(x, y)));
        }
        for (x, y) in [(4.7, 1.0), (0.0, 3.0)] {
            assert!(!geometry.intersects(&Point::new(x, y)));
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
