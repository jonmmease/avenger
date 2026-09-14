use crate::lyon_utils::IntoGeoType;
use crate::GeometryInstance;
use avenger_common::types::SymbolShape;
use avenger_scenegraph::marks::area::SceneAreaMark;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::image::SceneImageMark;
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::rule::SceneRuleMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::marks::text::{text_origin, SceneTextMark};
use avenger_scenegraph::marks::trail::SceneTrailMark;
use avenger_scenegraph::marks::{arc::SceneArcMark, mark::MarkInstance};
use avenger_typst_label::LabelEngine;
use geo::{Rotate, Scale, Translate};
use geo_types::{coord, Geometry, Rect};
use itertools::izip;
use lyon_algorithms::aabb::bounding_box;
use lyon_path::{
    math::{Angle, Transform, Vector},
    Event,
};
use rstar::{Envelope, AABB};
use std::iter::once;

/// Geometry of marks that draw no text.
pub trait MarkGeometryUtils {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_>;

    fn bounding_box(&self) -> AABB<[f32; 2]> {
        envelope(self.geometry_iter(Vec::new(), [0.0, 0.0]))
    }
}

/// Geometry of marks that can draw text, which the engine measures: text marks, and the groups
/// and marks that can hold them.
pub trait TextGeometryUtils {
    /// The mark's geometry, with its text measured by the engine.
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &LabelEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_>;

    /// The box around the mark's geometry.
    fn bounding_box(&self, text_engine: &LabelEngine) -> AABB<[f32; 2]> {
        envelope(self.geometry_iter(Vec::new(), [0.0, 0.0], text_engine))
    }
}

/// The box around some geometry, or an empty box at the origin.
fn envelope(geometry: impl Iterator<Item = GeometryInstance>) -> AABB<[f32; 2]> {
    geometry
        .map(|g| g.envelope())
        .reduce(|a, b| a.merged(&b))
        .unwrap_or(AABB::from_corners([0.0, 0.0], [0.0, 0.0]))
}

impl MarkGeometryUtils for SceneArcMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let name = self.name.clone();
        Box::new(
            izip!(
                self.indices_iter(),
                self.transformed_path_iter(origin),
                self.stroke_width_iter()
            )
            .map(move |(id, path, stroke_width)| {
                let half_stroke_width = stroke_width / 2.0;
                let geometry = path.as_geo_type(half_stroke_width, true);
                GeometryInstance {
                    mark_instance: MarkInstance {
                        name: name.clone(),
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                    },
                    interactive: self.interactive,
                    geometry,
                    reach: half_stroke_width,
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
        let name = self.name.clone();
        Box::new(once(GeometryInstance {
            mark_instance: MarkInstance {
                name: name.clone(),
                mark_path: mark_path.clone(),
                instance_index: None,
            },
            interactive: self.interactive,
            geometry: path.as_geo_type(half_stroke_width, true),
            reach: half_stroke_width,
        }))
    }
}

impl MarkGeometryUtils for SceneImageMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let name = self.name.clone();
        Box::new(
            izip!(self.indices_iter(), self.transformed_path_iter(origin)).map(
                move |(id, path)| {
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
                        interactive: self.interactive,
                        geometry,
                        reach: half_stroke_width,
                    }
                },
            ),
        )
    }
}

impl MarkGeometryUtils for avenger_scenegraph::marks::warped_image::SceneWarpedImageMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let Some([min_x, min_y, max_x, max_y]) = self.bounds(origin) else {
            return Box::new(std::iter::empty());
        };
        let instance = GeometryInstance {
            mark_instance: MarkInstance {
                name: self.name.clone(),
                mark_path,
                instance_index: Some(0),
            },
            interactive: self.interactive,
            geometry: Geometry::<f32>::Rect(Rect::new(
                coord!(x: min_x, y: min_y),
                coord!(x: max_x, y: max_y),
            )),
            reach: 0.0,
        };
        Box::new(std::iter::once(instance))
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
        let name = self.name.clone();
        Box::new(once(GeometryInstance {
            mark_instance: MarkInstance {
                name: name.clone(),
                mark_path: mark_path.clone(),
                instance_index: None,
            },
            interactive: self.interactive,
            geometry: path.as_geo_type(half_stroke_width, false),
            reach: half_stroke_width,
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
        let name = self.name.clone();
        Box::new(
            izip!(self.indices_iter(), self.transformed_path_iter(origin)).map(
                move |(id, path)| {
                    let geometry = path.filled_geo_type(0.1, self.fill_rule);
                    GeometryInstance {
                        mark_instance: MarkInstance {
                            name: name.clone(),
                            mark_path: mark_path.clone(),
                            instance_index: Some(id),
                        },
                        interactive: self.interactive,
                        geometry,
                        reach: half_stroke_width,
                    }
                },
            ),
        )
    }
}

impl MarkGeometryUtils for SceneRectMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
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
                .map(move |(id, x, y, x2, y2, stroke_width)| {
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
                        interactive: self.interactive,
                        geometry,
                        reach: *stroke_width / 2.0,
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
                .map(move |(id, path, stroke_width)| {
                    let half_stroke_width = stroke_width / 2.0;
                    let geometry = path.as_geo_type(0.1, true);
                    GeometryInstance {
                        mark_instance: MarkInstance {
                            name: name.clone(),
                            mark_path: mark_path.clone(),
                            instance_index: Some(id),
                        },
                        interactive: self.interactive,
                        geometry,
                        reach: half_stroke_width,
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
        let name = self.name.clone();
        Box::new(
            izip!(
                self.indices_iter(),
                self.transformed_path_iter(origin),
                self.stroke_width_iter(),
            )
            .map(move |(id, path, stroke_width)| {
                let half_stroke_width = stroke_width / 2.0;
                let geometry = path.as_geo_type(0.1, false);
                GeometryInstance {
                    mark_instance: MarkInstance {
                        name: name.clone(),
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                    },
                    interactive: self.interactive,
                    geometry,
                    reach: half_stroke_width,
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
        let name = self.name.clone();
        // Straight-edged shapes are exact as polygons at any size, so each is flattened once.
        let unit_polygons: Vec<Option<Geometry<f32>>> = self
            .shapes
            .iter()
            .map(|shape| match shape {
                SymbolShape::Path(path) if !has_curves(path) => {
                    Some(path.filled_geo_type(0.1, self.fill_rule))
                }
                _ => None,
            })
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
            .map(move |(instance_idx, x, y, size, angle, shape_idx)| {
                let center = [x + origin[0], y + origin[1]];
                let scale = size.sqrt();
                let (geometry, reach) = match (&self.shapes[*shape_idx], &unit_polygons[*shape_idx])
                {
                    // A circle is exact as its center, reaching as far as its radius.
                    (SymbolShape::Circle, _) => (
                        Geometry::Point(geo::Point::new(center[0], center[1])),
                        scale / 2.0 + half_stroke_width,
                    ),
                    (SymbolShape::Path(_), Some(unit)) => (
                        unit.scale_around_point(scale, scale, geo::Point::new(0.0, 0.0))
                            .rotate_around_point(*angle, geo::Point::new(0.0, 0.0))
                            .translate(center[0], center[1]),
                        half_stroke_width,
                    ),
                    // A curved shape is flattened at the size it draws at.
                    (SymbolShape::Path(path), None) => {
                        let transform = Transform::scale(scale, scale)
                            .then_rotate(Angle::degrees(*angle))
                            .then_translate(Vector::new(center[0], center[1]));
                        (
                            path.clone()
                                .transformed(&transform)
                                .filled_geo_type(0.1, self.fill_rule),
                            half_stroke_width,
                        )
                    }
                };

                GeometryInstance {
                    mark_instance: MarkInstance {
                        name: name.clone(),
                        mark_path: mark_path.clone(),
                        instance_index: Some(instance_idx),
                    },
                    interactive: self.interactive,
                    geometry,
                    reach,
                }
            }),
        )
    }
}

/// Whether a path has curves, which flattening approximates.
fn has_curves(path: &lyon_path::Path) -> bool {
    path.iter()
        .any(|event| matches!(event, Event::Quadratic { .. } | Event::Cubic { .. }))
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
            mark_instance: MarkInstance {
                name: self.name.clone(),
                mark_path: mark_path.clone(),
                instance_index: None,
            },
            interactive: self.interactive,
            geometry,
            reach: 0.0,
        }))
    }
}

impl TextGeometryUtils for SceneTextMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &LabelEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let instances: Vec<_> = izip!(self.indices_iter(), self.labels())
            .filter_map(|(id, label)| {
                // A label that doesn't lay out draws nothing, so it has no geometry.
                let bounds = text_engine.bounds(&label.label).ok()?;
                let anchor = [label.position[0] + origin[0], label.position[1] + origin[1]];
                let [left, top] = text_origin(&bounds, anchor, label.align, label.baseline);
                let rect = Rect::new(
                    coord!(x: left, y: top),
                    coord!(x: left + bounds.width, y: top + bounds.height),
                );
                Some(GeometryInstance {
                    mark_instance: MarkInstance {
                        name: self.name.clone(),
                        mark_path: mark_path.clone(),
                        instance_index: Some(id),
                    },
                    interactive: self.interactive,
                    geometry: Geometry::Rect(rect)
                        .rotate_around_point(label.angle, geo::Point::new(anchor[0], anchor[1])),
                    reach: 1.0,
                })
            })
            .collect();
        Box::new(instances.into_iter())
    }
}

impl TextGeometryUtils for SceneGroup {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &LabelEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let origin = [origin[0] + self.origin[0], origin[1] + self.origin[1]];
        // Collected, so that the iterator doesn't borrow the engine.
        let instances: Vec<_> = self
            .marks
            .iter()
            .enumerate()
            .flat_map(|(mark_index, mark)| {
                let mut mark_path = mark_path.clone();
                mark_path.push(mark_index);
                mark.geometry_iter(mark_path, origin, text_engine)
            })
            .collect();
        Box::new(instances.into_iter())
    }
}

impl TextGeometryUtils for SceneMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &LabelEngine,
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
            SceneMark::Text(mark) => mark.geometry_iter(mark_path, origin, text_engine),
            SceneMark::Image(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::WarpedImage(mark) => mark.geometry_iter(mark_path, origin),
            SceneMark::Group(mark) => mark.geometry_iter(mark_path, origin, text_engine),
        }
    }
}
