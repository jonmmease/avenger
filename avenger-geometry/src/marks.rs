use crate::lyon_utils::IntoGeoType;
use crate::GeometryInstance;
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
use avenger_scenegraph::marks::{arc::SceneArcMark, mark::MarkInstance};
use avenger_text::TextEngine;
use geo::{Rotate, Scale, Translate};
use geo_types::{coord, Geometry, Rect};
use itertools::izip;
use lyon_algorithms::aabb::bounding_box;
use rstar::{Envelope, RTreeObject, AABB};
use std::iter::once;

pub trait MarkGeometryUtils {
    /// The mark's geometry, with its text measured by the engine.
    fn geometry_iter<'a>(
        &'a self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &'a TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + 'a>;

    /// The box around the mark's geometry.
    fn bounding_box(&self, text_engine: &TextEngine) -> AABB<[f32; 2]> {
        self.geometry_iter(Vec::new(), [0.0, 0.0], text_engine)
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
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let name = self.name.clone();
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
                    mark_instance: MarkInstance {
                        name: name.clone(),
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                    },
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
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let path = self.transformed_path(origin);
        let half_stroke_width = self.stroke_width / 2.0;
        let name = self.name.clone();
        Box::new(once(GeometryInstance {
            mark_instance: MarkInstance {
                name: name.clone(),
                mark_path: mark_path.clone(),
                instance_index: None,
            },
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
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let name = self.name.clone();
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
                        mark_instance: MarkInstance {
                            name: name.clone(),
                            mark_path: mark_path.clone(),
                            instance_index: Some(id),
                        },
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
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let path = self.transformed_path(origin);
        let half_stroke_width = self.stroke_width / 2.0;
        let name = self.name.clone();
        Box::new(once(GeometryInstance {
            mark_instance: MarkInstance {
                name: name.clone(),
                mark_path: mark_path.clone(),
                instance_index: None,
            },
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
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let half_stroke_width = self.stroke_width.unwrap_or(0.0) / 2.0;
        let name = self.name.clone();
        Box::new(
            izip!(self.indices_iter(), self.transformed_path_iter(origin))
                .enumerate()
                .map(move |(z_index, (id, path))| {
                    let geometry = path.as_geo_type(0.1, true);
                    GeometryInstance {
                        mark_instance: MarkInstance {
                            name: name.clone(),
                            mark_path: mark_path.clone(),
                            instance_index: Some(id),
                        },
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
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let name = self.name.clone();
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
                    let x0 = f32::min(*x, x2) + origin[0];
                    let x1 = f32::max(*x, x2) + origin[0];
                    let y0 = f32::min(*y, y2) + origin[1];
                    let y1 = f32::max(*y, y2) + origin[1];

                    let geometry = Geometry::Rect(Rect::<f32>::new(
                        coord!(x: x0, y: y0),
                        coord!(x: x1, y: y1),
                    ));
                    GeometryInstance {
                        mark_instance: MarkInstance {
                            name: name.clone(),
                            mark_path: mark_path.clone(),
                            instance_index: Some(id),
                        },
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
                        mark_instance: MarkInstance {
                            name: name.clone(),
                            mark_path: mark_path.clone(),
                            instance_index: Some(id),
                        },
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
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let name = self.name.clone();
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
                    mark_instance: MarkInstance {
                        name: name.clone(),
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                    },
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
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let name = self.name.clone();
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
                        .translate(x + origin[0], y + origin[1]);

                    GeometryInstance {
                        mark_instance: MarkInstance {
                            name: name.clone(),
                            mark_path: mark_path.clone(),
                            instance_index: Some(instance_idx),
                        },
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
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let path = self.transformed_path(origin);
        let geometry = path.trail_as_geo_type(0.1, 0);
        Box::new(once(GeometryInstance {
            mark_instance: MarkInstance {
                name: self.name.clone(),
                mark_path: mark_path.clone(),
                instance_index: None,
            },
            z_index: 0,
            geometry,
            half_stroke_width: 0.0,
        }))
    }
}

impl MarkGeometryUtils for SceneTextMark {
    fn geometry_iter<'a>(
        &'a self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &'a TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + 'a> {
        let formatters = self.formatters();
        let instances: Vec<_> = izip!(self.indices_iter(), self.labels(&formatters))
            .enumerate()
            .map(|(z_index, (id, label))| {
                let bounds =
                    text_engine.measure_bounds_with_plain_fallback_or_approx(&label.config);
                let anchor = [label.position[0] + origin[0], label.position[1] + origin[1]];
                let [left, top] = bounds.calculate_origin(anchor, &label.align, &label.baseline);
                let rect = Rect::new(
                    coord!(x: left, y: top),
                    coord!(x: left + bounds.width, y: top + bounds.height),
                );
                GeometryInstance {
                    mark_instance: MarkInstance {
                        name: self.name.clone(),
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                    },
                    z_index,
                    geometry: Geometry::Rect(rect)
                        .rotate_around_point(label.angle, geo::Point::new(anchor[0], anchor[1])),
                    half_stroke_width: 1.0,
                }
            })
            .collect();
        Box::new(instances.into_iter())
    }
}

impl MarkGeometryUtils for SceneGroup {
    fn geometry_iter<'a>(
        &'a self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &'a TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + 'a> {
        let origin = [origin[0] + self.origin[0], origin[1] + self.origin[1]];
        Box::new(
            self.marks
                .iter()
                .enumerate()
                .flat_map(move |(mark_index, mark)| {
                    let mut mark_path = mark_path.clone();
                    mark_path.push(mark_index);
                    mark.geometry_iter(mark_path, origin, text_engine)
                }),
        )
    }
}

impl MarkGeometryUtils for SceneMark {
    fn geometry_iter<'a>(
        &'a self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &'a TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + 'a> {
        match self {
            SceneMark::Arc(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Area(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Path(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Symbol(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Line(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Trail(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Rect(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Rule(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Text(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Image(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Group(mark) => mark.geometry_iter(mark_path, origin, text_engine),
        }
    }
}
