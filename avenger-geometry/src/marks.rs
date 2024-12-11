use crate::lyon_to_geo::IntoGeoType;
use crate::GeometryInstance;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::area::SceneAreaMark;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::image::SceneImageMark;
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::rule::SceneRuleMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::marks::trail::SceneTrailMark;
use avenger_text::rasterization::TextRasterizer;
use avenger_text::rasterization::{default_rasterizer, TextRasterizationConfig};
use geo::{BooleanOps, Rotate, Scale, Translate};
use geo_types::{coord, Geometry, Rect};
use itertools::izip;
use lyon_algorithms::aabb::bounding_box;
use rstar::{Envelope, RTreeObject, AABB};
use std::iter::once;

pub trait MarkGeometryUtils {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_>;

    fn bounding_box(&self) -> AABB<[f32; 2]> {
        self.geometry_iter(Vec::new(), [0.0, 0.0])
            .map(|g| g.envelope())
            .reduce(|a, b| a.merged(&b))
            .unwrap_or(AABB::from_corners([0.0, 0.0], [0.0, 0.0]))
    }
}

impl MarkGeometryUtils for SceneArcMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        Box::new(
            izip!(
                self.indices_iter(),
                self.transformed_path_iter(origin),
                self.stroke_width_iter()
            )
            .enumerate()
            .map(move |(z_index, (id, path, stroke_width))| {
                let half_stroke_width = stroke_width / 2.0;
                let geometry = path.as_geo_type(half_stroke_width, true);
                GeometryInstance {
                    mark_path: mark_path.clone(),
                    instance_index: Some(id),
                    z_index,
                    geometry,
                    half_stroke_width,
                }
            }),
        )
    }
}

impl MarkGeometryUtils for SceneAreaMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let path = self.transformed_path(origin);
        let half_stroke_width = self.stroke_width / 2.0;
        Box::new(once(GeometryInstance {
            mark_path: mark_path.clone(),
            instance_index: None,
            z_index: 0,
            geometry: path.as_geo_type(half_stroke_width, true),
            half_stroke_width,
        }))
    }
}

impl MarkGeometryUtils for SceneImageMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        Box::new(
            izip!(self.indices_iter(), self.transformed_path_iter(origin))
                .enumerate()
                .map(move |(z_index, (id, path))| {
                    let half_stroke_width = 0.0;

                    let bbox = bounding_box(&path);
                    let geometry = Geometry::<f32>::Rect(Rect::new(
                        coord!(x: bbox.min.x, y: bbox.min.y),
                        coord!(x: bbox.max.x, y: bbox.max.y),
                    ));

                    GeometryInstance {
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                        z_index,
                        geometry,
                        half_stroke_width,
                    }
                }),
        )
    }
}

impl MarkGeometryUtils for SceneLineMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let path = self.transformed_path(origin);
        let half_stroke_width = self.stroke_width / 2.0;
        Box::new(once(GeometryInstance {
            mark_path: mark_path.clone(),
            instance_index: None,
            z_index: 0,
            geometry: path.as_geo_type(half_stroke_width, false),
            half_stroke_width,
        }))
    }
}

impl MarkGeometryUtils for ScenePathMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let half_stroke_width = self.stroke_width.unwrap_or(0.0) / 2.0;
        Box::new(
            izip!(self.indices_iter(), self.transformed_path_iter(origin))
                .enumerate()
                .map(move |(z_index, (id, path))| {
                    let geometry = path.as_geo_type(0.1, true);
                    GeometryInstance {
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                        z_index,
                        geometry,
                        half_stroke_width,
                    }
                }),
        )
    }
}

impl MarkGeometryUtils for SceneRectMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        if self.corner_radius.equals_scalar(0.0) {
            // Simple case where we don't need to build lyon paths first
            Box::new(
                izip!(
                    self.indices_iter(),
                    self.x_iter(),
                    self.y_iter(),
                    self.x2_iter(),
                    self.y2_iter(),
                    self.stroke_width_iter()
                )
                .enumerate()
                .map(move |(z_index, (id, x, y, x2, y2, stroke_width))| {
                    // Create rect geometry
                    let x0 = f32::min(*x, x2);
                    let x1 = f32::max(*x, x2);
                    let y0 = f32::min(*y, y2);
                    let y1 = f32::max(*y, y2);

                    let geometry = Geometry::Rect(Rect::<f32>::new(
                        coord!(x: x0 + origin[0], y: y0 + origin[1]),
                        coord!(x: x1 + origin[0], y: y1 + origin[1]),
                    ));
                    GeometryInstance {
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                        z_index,
                        geometry,
                        half_stroke_width: *stroke_width / 2.0,
                    }
                }),
            )
        } else {
            // General case
            Box::new(
                izip!(
                    self.indices_iter(),
                    self.transformed_path_iter(origin),
                    self.stroke_width_iter()
                )
                .enumerate()
                .map(move |(z_index, (id, path, stroke_width))| {
                    let half_stroke_width = stroke_width / 2.0;
                    let geometry = path.as_geo_type(0.1, true);
                    GeometryInstance {
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                        z_index,
                        geometry,
                        half_stroke_width,
                    }
                }),
            )
        }
    }
}

impl MarkGeometryUtils for SceneRuleMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        Box::new(
            izip!(
                self.indices_iter(),
                self.transformed_path_iter(origin),
                self.stroke_width_iter(),
            )
            .enumerate()
            .map(move |(z_index, (id, path, stroke_width))| {
                let half_stroke_width = stroke_width / 2.0;
                let geometry = path.as_geo_type(0.1, false);
                GeometryInstance {
                    mark_path: mark_path.clone(),
                    instance_index: Some(id),
                    z_index,
                    geometry,
                    half_stroke_width,
                }
            }),
        )
    }
}

impl MarkGeometryUtils for SceneSymbolMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let symbol_geometries: Vec<_> = self
            .shapes
            .iter()
            .map(|symbol| symbol.as_path().as_geo_type(0.1, true))
            .collect();
        let half_stroke_width = self.stroke_width.unwrap_or(0.0) / 2.0;
        Box::new(
            izip!(
                self.indices_iter(),
                self.x_iter(),
                self.y_iter(),
                self.size_iter(),
                self.angle_iter(),
                self.shape_index_iter()
            )
            .enumerate()
            .map(
                move |(z_index, (instance_idx, x, y, size, angle, shape_idx))| {
                    let geometry = symbol_geometries[*shape_idx]
                        .clone()
                        .scale(size.sqrt())
                        .rotate_around_point(angle.to_radians(), geo::Point::new(0.0, 0.0))
                        .translate(*x + origin[0], *y + origin[1]);

                    GeometryInstance {
                        mark_path: mark_path.clone(),
                        instance_index: Some(instance_idx),
                        z_index,
                        geometry,
                        half_stroke_width,
                    }
                },
            ),
        )
    }
}

impl MarkGeometryUtils for SceneTrailMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let path = self.transformed_path(origin);
        let geometry = path.trail_as_geo_type(0.1, 0);
        Box::new(once(GeometryInstance {
            mark_path: mark_path.clone(),
            instance_index: None,
            z_index: 0,
            geometry,
            half_stroke_width: 0.0,
        }))
    }
}

impl MarkGeometryUtils for SceneTextMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let rasterizer = default_rasterizer();
        Box::new(
            izip!(
                self.indices_iter(),
                self.text_iter(),
                self.x_iter(),
                self.y_iter(),
                self.angle_iter(),
                self.font_iter(),
                self.font_size_iter(),
                self.font_weight_iter(),
                self.font_style_iter(),
                self.align_iter(),
                self.baseline_iter()
            )
            .enumerate()
            .map(
                move |(
                    z_index,
                    (
                        id,
                        text,
                        x,
                        y,
                        angle,
                        font,
                        font_size,
                        font_weight,
                        font_style,
                        align,
                        baseline,
                    ),
                )| {
                    let config = TextRasterizationConfig {
                        text: text,
                        font: font,
                        font_size: *font_size,
                        font_weight: font_weight,
                        font_style: font_style,
                        color: &[0.0, 0.0, 0.0, 1.0],
                        limit: 0.0,
                    };

                    let text_buffer = rasterizer
                        .rasterize(&config, 1.0, &Default::default())
                        .unwrap();

                    let local_origin = text_buffer.text_bounds.calculate_origin(
                        [*x + origin[0], *y + origin[1]],
                        align,
                        baseline,
                    );

                    // Check if we have path data for every glyph
                    let has_any_path_data = text_buffer
                        .glyphs
                        .iter()
                        .any(|(glyph_data, _)| glyph_data.path.is_some());

                    // Build up the text polygon by unioning the glyph bounding boxes
                    let mut text_poly = geo::MultiPolygon::<f32>::new(vec![]);

                    let geometry = if has_any_path_data {
                        for (glyph_data, phys_pos) in text_buffer.glyphs {
                            let glyph_bbox_poly = if let Some(path) = &glyph_data.path {
                                match path.as_geo_type(0.0, true) {
                                    geo::Geometry::Polygon(poly) => geo::MultiPolygon::new(vec![poly]),
                                    geo::Geometry::MultiPolygon(mpoly) => mpoly,
                                    g => panic!("Expected polygon or multipolygon: {:?}", g),
                                }
                            } else {
                                let glyph_bbox = glyph_data.bbox;
                                geo::MultiPolygon::new(vec![geo::Polygon::new(
                                    geo::LineString::new(vec![
                                        geo::Coord {
                                            x: glyph_bbox.left as f32,
                                            y: -glyph_bbox.top as f32,
                                        },
                                        geo::Coord {
                                            x: glyph_bbox.left as f32 + glyph_bbox.width as f32,
                                            y: -glyph_bbox.top as f32,
                                        },
                                        geo::Coord {
                                            x: glyph_bbox.left as f32 + glyph_bbox.width as f32,
                                            y: -glyph_bbox.top as f32 + glyph_bbox.height as f32,
                                        },
                                        geo::Coord {
                                            x: glyph_bbox.left as f32,
                                            y: -glyph_bbox.top as f32 + glyph_bbox.height as f32,
                                        },
                                        geo::Coord {
                                            x: glyph_bbox.left as f32,
                                            y: -glyph_bbox.top as f32,
                                        },
                                    ]),
                                    vec![],
                                )])
                            }
                            .translate(
                                phys_pos.x + local_origin[0],
                                phys_pos.y + local_origin[1] + text_buffer.text_bounds.height,
                            );
                            text_poly = text_poly.union(&glyph_bbox_poly);
                        }
                        Geometry::MultiPolygon(text_poly)
                            .rotate_around_point(*angle, geo::Point::new(*x + origin[0], *y + origin[1]))
                    } else {
                        // text_buffer.text_bounds
                        let bounds = geo::Rect::new(
                            coord!(x: local_origin[0], y: local_origin[1]),
                            coord!(x: local_origin[0] + text_buffer.text_bounds.width, y: local_origin[1] + text_buffer.text_bounds.line_height),
                        );

                        Geometry::Rect(bounds)
                            .rotate_around_point(*angle, geo::Point::new(*x + origin[0], *y + origin[1]))
                    };

                    GeometryInstance {
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                        z_index,
                        geometry,
                        half_stroke_width: 0.0,
                    }
                },
            ),
        )
    }
}

impl MarkGeometryUtils for SceneGroup {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        Box::new(
            self.marks
                .iter()
                .enumerate()
                .flat_map(move |(mark_index, mark)| {
                    // Build up the mark path
                    let mut mark_path = mark_path.clone();
                    mark_path.push(mark_index);

                    // Compute absolute origin for group
                    let origin = [origin[0] + self.origin[0], origin[1] + self.origin[1]];
                    mark.geometry_iter(mark_path, origin)
                }),
        )
    }
}

impl MarkGeometryUtils for SceneMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        match self {
            SceneMark::Arc(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Area(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Path(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Symbol(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Line(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Trail(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Rect(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Rule(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Text(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Image(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Group(mark) => mark.geometry_iter(mark_path, origin),
        }
    }
}
