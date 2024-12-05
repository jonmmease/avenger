use geo_types::{Coord, Geometry, LineString, MultiLineString, MultiPolygon, Polygon};
use lyon_path::iterator::PathIterator;
use lyon_path::{Path, PathEvent};

pub trait IntoGeoType {
    /// Convert the path into a geo-types geometry
    ///
    /// # Arguments
    ///
    /// * `tolerance` - The tolerance to use when flattening curves
    /// * `filled` - If true, treat all paths as filled polygons by forcing closure.
    ///             If false, treat all paths as lines
    fn into_geo_type(self, tolerance: f32, filled: bool) -> Option<Geometry>;
}

impl IntoGeoType for Path {
    fn into_geo_type(self, tolerance: f32, filled: bool) -> Option<Geometry> {
        let mut current_line: Vec<Coord> = Vec::new();
        let mut lines: Vec<LineString> = Vec::new();
        let mut polygons: Vec<Polygon> = Vec::new();
        let mut current_start: Option<Coord> = None;

        // Flatten the path and collect coordinates
        for evt in self.iter().flattened(tolerance) {
            match evt {
                PathEvent::Begin { at } => {
                    if !current_line.is_empty() {
                        let line = std::mem::take(&mut current_line);
                        if filled && line.len() >= 3 {
                            // In filled mode, force close any path with 3 or more points
                            let mut closed_line = line;
                            if closed_line.first() != closed_line.last() {
                                closed_line.push(closed_line[0].clone());
                            }
                            polygons.push(Polygon::new(
                                LineString::new(closed_line),
                                vec![], // No interior rings
                            ));
                        } else {
                            lines.push(LineString::new(line));
                        }
                    }
                    let coord = Coord {
                        x: at.x as f64,
                        y: at.y as f64,
                    };
                    current_start = Some(coord.clone());
                    current_line.push(coord);
                }
                PathEvent::Line { to, .. } => {
                    current_line.push(Coord {
                        x: to.x as f64,
                        y: to.y as f64,
                    });
                }
                PathEvent::End {
                    close: _, first: _, ..
                } => {
                    if !current_line.is_empty() {
                        if filled && current_line.len() >= 3 {
                            // In filled mode, force close any path with 3 or more points
                            if current_line.first() != current_line.last() {
                                if let Some(start) = current_start.clone() {
                                    current_line.push(start);
                                }
                            }
                            polygons.push(Polygon::new(
                                LineString::new(std::mem::take(&mut current_line)),
                                vec![], // No interior rings
                            ));
                        } else {
                            lines.push(LineString::new(std::mem::take(&mut current_line)));
                        }
                    }
                    current_start = None;
                }
                _ => {} // We don't need to handle curves as we're using flattened iterator
            }
        }

        // Return the appropriate geometry type based on what we collected
        if filled {
            match polygons.len() {
                0 => None,
                1 => Some(Geometry::Polygon(polygons.into_iter().next().unwrap())),
                _ => Some(Geometry::MultiPolygon(MultiPolygon(polygons))),
            }
        } else {
            match lines.len() {
                0 => None,
                1 => Some(Geometry::LineString(lines.into_iter().next().unwrap())),
                _ => Some(Geometry::MultiLineString(MultiLineString(lines))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::approx_eq;
    use geo_types::Coord;
    use lyon_path::{math::point, Path};

    fn assert_coords_eq(a: &Coord, b: &Coord) {
        assert!(
            approx_eq!(f64, a.x, b.x, ulps = 2),
            "x coordinates differ: {} != {}",
            a.x,
            b.x
        );
        assert!(
            approx_eq!(f64, a.y, b.y, ulps = 2),
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
        let geometry = path.into_geo_type(0.1, false).unwrap();

        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 3);
                assert_coords_eq(&coords[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(&coords[1], &Coord { x: 1.0, y: 0.0 });
                assert_coords_eq(&coords[2], &Coord { x: 1.0, y: 1.0 });
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
        let geometry = path.clone().into_geo_type(0.1, true).unwrap();
        match geometry {
            Geometry::Polygon(polygon) => {
                let coords: Vec<_> = polygon.exterior().coords().collect();
                assert_eq!(coords.len(), 5); // 4 unique points + closing point
                assert_coords_eq(&coords[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(&coords[1], &Coord { x: 1.0, y: 0.0 });
                assert_coords_eq(&coords[2], &Coord { x: 1.0, y: 1.0 });
                assert_coords_eq(&coords[3], &Coord { x: 0.0, y: 1.0 });
                assert_coords_eq(&coords[4], &coords[0]); // Should close the polygon
            }
            _ => panic!("Expected Polygon"),
        }

        // Test unfilled mode (should be closed linestring)
        let geometry = path.into_geo_type(0.1, false).unwrap();
        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 4); // 4 unique points + closing point
                assert_coords_eq(&coords[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(&coords[1], &Coord { x: 1.0, y: 0.0 });
                assert_coords_eq(&coords[2], &Coord { x: 1.0, y: 1.0 });
                assert_coords_eq(&coords[3], &Coord { x: 0.0, y: 1.0 });
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
        let geometry = path.into_geo_type(0.1, false).unwrap();

        match geometry {
            Geometry::MultiLineString(multi_line) => {
                assert_eq!(multi_line.0.len(), 2);

                let first_line: Vec<_> = multi_line.0[0].coords().collect();
                assert_eq!(first_line.len(), 2);
                assert_coords_eq(&first_line[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(&first_line[1], &Coord { x: 1.0, y: 0.0 });

                let second_line: Vec<_> = multi_line.0[1].coords().collect();
                assert_eq!(second_line.len(), 2);
                assert_coords_eq(&second_line[0], &Coord { x: 0.0, y: 1.0 });
                assert_coords_eq(&second_line[1], &Coord { x: 1.0, y: 1.0 });
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
        let geometry = path.clone().into_geo_type(0.1, true).unwrap();
        match geometry {
            Geometry::MultiPolygon(multi_polygon) => {
                assert_eq!(multi_polygon.0.len(), 2);

                // Check first polygon
                let first_poly: Vec<_> = multi_polygon.0[0].exterior().coords().collect();
                assert_eq!(first_poly.len(), 5);
                assert_coords_eq(&first_poly[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(&first_poly[4], &first_poly[0]); // Should close

                // Check second polygon
                let second_poly: Vec<_> = multi_polygon.0[1].exterior().coords().collect();
                assert_eq!(second_poly.len(), 5);
                assert_coords_eq(&second_poly[0], &Coord { x: 2.0, y: 0.0 });
                assert_coords_eq(&second_poly[4], &second_poly[0]); // Should close
            }
            _ => panic!("Expected MultiPolygon"),
        }

        // Test unfilled mode (should be multi-linestring)
        let geometry = path.into_geo_type(0.1, false).unwrap();
        match geometry {
            Geometry::MultiLineString(multi_line) => {
                assert_eq!(multi_line.0.len(), 2);

                // Check first linestring
                let first_line: Vec<_> = multi_line.0[0].coords().collect();
                assert_eq!(first_line.len(), 4);
                assert_coords_eq(&first_line[0], &Coord { x: 0.0, y: 0.0 });
                assert_coords_eq(&first_line[3], &Coord { x: 0.0, y: 1.0 }); // Should close

                // Check second linestring
                let second_line: Vec<_> = multi_line.0[1].coords().collect();
                assert_eq!(second_line.len(), 4);
                assert_coords_eq(&second_line[0], &Coord { x: 2.0, y: 0.0 });
                assert_coords_eq(&second_line[3], &Coord { x: 2.0, y: 1.0 }); // Should close
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
        let geometry = path.into_geo_type(0.1, false).unwrap();

        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert!(
                    coords.len() > 2,
                    "Curve should be flattened into multiple points"
                );
                assert_coords_eq(&coords[0], &Coord { x: 0.0, y: 0.0 });
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
        let geometry = path.into_geo_type(0.1, false).unwrap();

        match geometry {
            Geometry::LineString(line) => {
                let coords: Vec<_> = line.coords().collect();
                assert_eq!(coords.len(), 1);
                assert_coords_eq(&coords[0], &Coord { x: 0.0, y: 0.0 });
            }
            _ => panic!("Expected LineString"),
        }
    }
}
