use std::iter::once;

use avenger_scenegraph::marks::{
    arc::SceneArcMark,
    area::SceneAreaMark,
    group::SceneGroup,
    image::SceneImageMark,
    line::SceneLineMark,
    mark::{MarkInstance, SceneMark},
    path::ScenePathMark,
    rect::SceneRectMark,
    rule::SceneRuleMark,
    symbol::SceneSymbolMark,
    text::SceneTextMark,
    text_leader::{
        compute_text_leader_geometry, TextLeaderArrowhead, TextLeaderGeometry,
        TextLeaderGeometryInput, TextLeaderPath,
    },
    trail::SceneTrailMark,
};
use avenger_text::{measurement::TextMeasurementConfig, TextEngine};
use geo::{Rotate, Scale, Translate};
use geo_types::{coord, Geometry, GeometryCollection, LineString, Polygon, Rect};
use itertools::izip;
use lyon_algorithms::aabb::bounding_box;
use rstar::{Envelope, RTreeObject, AABB};

use crate::{lyon_utils::IntoGeoType, GeometryInstance};

pub trait MarkGeometryUtils {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_>;

    /// Use the same configured text engine for layout, rendering, and picking.
    fn geometry_iter_with_text_engine(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        _text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        self.geometry_iter(mark_path, origin)
    }

    fn bounding_box_with_text_engine(&self, text_engine: &TextEngine) -> AABB<[f32; 2]> {
        self.geometry_iter_with_text_engine(Vec::new(), [0.0, 0.0], text_engine)
            .map(|g| g.envelope())
            .reduce(|a, b| a.merged(&b))
            .unwrap_or(AABB::from_corners([0.0, 0.0], [0.0, 0.0]))
    }

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
                    interactive: self.interactive,
                    instance_order: z_index,
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
        let name = self.name.clone();
        Box::new(once(GeometryInstance {
            mark_instance: MarkInstance {
                name: name.clone(),
                mark_path: mark_path.clone(),
                instance_index: None,
            },
            interactive: self.interactive,
            instance_order: 0,
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
                        interactive: self.interactive,
                        instance_order: z_index,
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
        let name = self.name.clone();
        Box::new(once(GeometryInstance {
            mark_instance: MarkInstance {
                name: name.clone(),
                mark_path: mark_path.clone(),
                instance_index: None,
            },
            interactive: self.interactive,
            instance_order: 0,
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
                        interactive: self.interactive,
                        instance_order: z_index,
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
                        interactive: self.interactive,
                        instance_order: z_index,
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
                        interactive: self.interactive,
                        instance_order: z_index,
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
                    interactive: self.interactive,
                    instance_order: z_index,
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
                        interactive: self.interactive,
                        instance_order: z_index,
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
            mark_instance: MarkInstance {
                name: self.name.clone(),
                mark_path: mark_path.clone(),
                instance_index: None,
            },
            interactive: self.interactive,
            instance_order: 0,
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
        self.geometry_iter_with_text_engine(mark_path, origin, &avenger_text::default_text_engine())
    }

    fn geometry_iter_with_text_engine(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let name = self.name.clone();
        let mut instances = Vec::new();
        for (
            z_index,
            (
                id,
                text,
                target,
                label,
                defined,
                angle,
                font,
                font_size,
                font_weight,
                font_style,
                align,
                baseline,
                limit,
                leader,
                leader_stroke_width,
                leader_label_padding,
                leader_target_radius,
                leader_min_length,
                leader_shape,
                leader_arrow,
                leader_arrow_length,
                leader_arrow_width,
            ),
        ) in izip!(
            self.indices_iter(),
            self.text_iter(),
            self.target_position_iter(),
            self.label_position_iter(),
            self.defined_iter(),
            self.angle_iter(),
            self.font_iter(),
            self.font_size_iter(),
            self.font_weight_iter(),
            self.font_style_iter(),
            self.align_iter(),
            self.baseline_iter(),
            self.limit_iter(),
            self.leader_iter(),
            self.leader_stroke_width_iter(),
            self.leader_label_padding_iter(),
            self.leader_target_radius_iter(),
            self.leader_min_length_iter(),
            self.leader_shape_iter(),
            self.leader_arrow_iter(),
            self.leader_arrow_length_iter(),
            self.leader_arrow_width_iter()
        )
        .enumerate()
        {
            if !*defined {
                continue;
            }

            let config = TextMeasurementConfig {
                text,
                font,
                font_size: *font_size,
                font_weight: *font_weight,
                font_style: *font_style,
                syntax_mode: self.text_syntax,
                params: &self.text_params,
                number_locale: self.number_locale.as_deref(),
                number_locale_specs: Some(&self.number_locale_specs),
                datetime_locale: self.datetime_locale.as_deref(),
                datetime_timezone: self.datetime_timezone.as_deref(),
                datetime_locale_specs: Some(&self.datetime_locale_specs),
            };

            let target = [target[0] + origin[0], target[1] + origin[1]];
            let label = [label[0] + origin[0], label[1] + origin[1]];
            let text_bounds = text_engine.measure_bounds_with_limit_or_approx(&config, *limit);
            let local_origin = text_bounds.calculate_origin(label, align, baseline);

            let bounds = Rect::new(
                coord!(x: local_origin[0], y: local_origin[1]),
                coord!(x: local_origin[0] + text_bounds.width, y: local_origin[1] + text_bounds.height),
            );

            let mut geometries = vec![Geometry::Rect(bounds)
                .rotate_around_point(*angle, geo::Point::new(label[0], label[1]))];
            let mut half_stroke_width: f32 = 1.0;

            if *leader {
                if let Some(leader_geometry) =
                    compute_text_leader_geometry(TextLeaderGeometryInput {
                        target,
                        label_anchor: label,
                        angle_degrees: *angle,
                        text_bounds: &text_bounds,
                        align,
                        baseline,
                        label_padding: *leader_label_padding,
                        target_radius: *leader_target_radius,
                        min_length: *leader_min_length,
                        shape: *leader_shape,
                        arrow: *leader_arrow,
                        arrow_length: *leader_arrow_length,
                        arrow_width: *leader_arrow_width,
                    })
                {
                    geometries.extend(text_leader_geometry_to_geo(&leader_geometry));
                    half_stroke_width = half_stroke_width.max(*leader_stroke_width / 2.0);
                }
            }

            let geometry = if geometries.len() == 1 {
                geometries.pop().unwrap()
            } else {
                Geometry::GeometryCollection(GeometryCollection(geometries))
            };

            instances.push(GeometryInstance {
                mark_instance: MarkInstance {
                    name: name.clone(),
                    mark_path: mark_path.clone(),
                    instance_index: Some(id),
                },
                interactive: self.interactive,
                instance_order: z_index,
                geometry,
                half_stroke_width,
            });
        }

        Box::new(instances.into_iter())
    }
}

fn text_leader_geometry_to_geo(geometry: &TextLeaderGeometry) -> Vec<Geometry<f32>> {
    let mut geometries = vec![text_leader_path_to_geo(&geometry.spine)];
    if let Some(arrowhead) = &geometry.arrowhead {
        geometries.push(text_leader_arrowhead_to_geo(arrowhead));
    }
    geometries
}

fn text_leader_path_to_geo(path: &TextLeaderPath) -> Geometry<f32> {
    match path {
        TextLeaderPath::Line { start, end } => Geometry::LineString(LineString::from(vec![
            point_tuple(*start),
            point_tuple(*end),
        ])),
        TextLeaderPath::Polyline { points } => Geometry::LineString(LineString::from(
            points.iter().copied().map(point_tuple).collect::<Vec<_>>(),
        )),
        TextLeaderPath::Cubic {
            start,
            ctrl1,
            ctrl2,
            end,
        } => Geometry::LineString(LineString::from(
            (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0;
                    point_tuple(cubic_point(*start, *ctrl1, *ctrl2, *end, t))
                })
                .collect::<Vec<_>>(),
        )),
    }
}

fn text_leader_arrowhead_to_geo(arrowhead: &TextLeaderArrowhead) -> Geometry<f32> {
    match arrowhead {
        TextLeaderArrowhead::Open { left, right } => {
            Geometry::GeometryCollection(GeometryCollection(vec![
                Geometry::LineString(LineString::from(vec![
                    point_tuple(left[0]),
                    point_tuple(left[1]),
                ])),
                Geometry::LineString(LineString::from(vec![
                    point_tuple(right[0]),
                    point_tuple(right[1]),
                ])),
            ]))
        }
        TextLeaderArrowhead::Triangle { points } => {
            let mut coords = points.iter().copied().map(point_tuple).collect::<Vec<_>>();
            coords.push(point_tuple(points[0]));
            Geometry::Polygon(Polygon::new(LineString::from(coords), vec![]))
        }
    }
}

fn cubic_point(
    start: [f32; 2],
    ctrl1: [f32; 2],
    ctrl2: [f32; 2],
    end: [f32; 2],
    t: f32,
) -> [f32; 2] {
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let t2 = t * t;
    [
        mt2 * mt * start[0] + 3.0 * mt2 * t * ctrl1[0] + 3.0 * mt * t2 * ctrl2[0] + t2 * t * end[0],
        mt2 * mt * start[1] + 3.0 * mt2 * t * ctrl1[1] + 3.0 * mt * t2 * ctrl2[1] + t2 * t * end[1],
    ]
}

fn point_tuple(point: [f32; 2]) -> (f32, f32) {
    (point[0], point[1])
}

impl MarkGeometryUtils for SceneGroup {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        self.geometry_iter_with_text_engine(mark_path, origin, &avenger_text::default_text_engine())
    }

    fn geometry_iter_with_text_engine(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let mut instances = Vec::new();
        for (mark_index, mark) in self.marks.iter().enumerate() {
            let mut mark_path = mark_path.clone();
            mark_path.push(mark_index);
            let origin = [origin[0] + self.origin[0], origin[1] + self.origin[1]];
            instances.extend(mark.geometry_iter_with_text_engine(mark_path, origin, text_engine));
        }
        Box::new(instances.into_iter())
    }

    fn bounding_box(&self) -> AABB<[f32; 2]> {
        self.bounding_box_with_text_engine(&avenger_text::default_text_engine())
    }

    fn bounding_box_with_text_engine(&self, text_engine: &TextEngine) -> AABB<[f32; 2]> {
        use avenger_scenegraph::marks::group::Clip;

        // If the group has a clip rect, use that as the bounding box
        match &self.clip {
            Clip::Rect {
                x,
                y,
                width,
                height,
            } => {
                // Clip coordinates are relative to the group's origin
                let min_x = self.origin[0] + x;
                let min_y = self.origin[1] + y;
                let max_x = min_x + width;
                let max_y = min_y + height;
                AABB::from_corners([min_x, min_y], [max_x, max_y])
            }
            _ => {
                // For other clip types or no clip, use the default implementation
                // which computes the union of children's bounding boxes
                self.geometry_iter_with_text_engine(Vec::new(), [0.0, 0.0], text_engine)
                    .map(|g| g.envelope())
                    .reduce(|a, b| a.merged(&b))
                    .unwrap_or(AABB::from_corners([0.0, 0.0], [0.0, 0.0]))
            }
        }
    }
}

impl MarkGeometryUtils for SceneMark {
    fn geometry_iter(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        self.geometry_iter_with_text_engine(mark_path, origin, &avenger_text::default_text_engine())
    }

    fn geometry_iter_with_text_engine(
        &self,
        mark_path: Vec<usize>,
        origin: [f32; 2],
        text_engine: &TextEngine,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        match self {
            SceneMark::Arc(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
            SceneMark::Area(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
            SceneMark::Path(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
            SceneMark::Symbol(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
            SceneMark::Line(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
            SceneMark::Trail(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
            SceneMark::Rect(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
            SceneMark::Rule(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
            SceneMark::Text(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
            SceneMark::Image(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }

            SceneMark::Group(mark) => {
                mark.geometry_iter_with_text_engine(mark_path, origin, text_engine)
            }
        }
    }
}
