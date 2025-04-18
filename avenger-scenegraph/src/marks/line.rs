use avenger_common::types::{ColorOrGradient, Gradient, StrokeCap, StrokeJoin};
use avenger_common::value::ScalarOrArray;
use lyon_algorithms::measure::{PathMeasurements, PathSampler, SampleType};
use lyon_path::{geom::point, Path};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneLineMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub defined: ScalarOrArray<bool>,
    pub stroke: ColorOrGradient,
    pub stroke_width: f32,
    pub stroke_cap: StrokeCap,
    pub stroke_join: StrokeJoin,
    pub stroke_dash: Option<Vec<f32>>,
    pub zindex: Option<i32>,
}

impl std::hash::Hash for SceneLineMark {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.clip.hash(state);
        self.len.hash(state);
        self.gradients.hash(state);
        self.x.hash(state);
        self.y.hash(state);
        self.defined.hash(state);
        self.stroke.hash(state);
        OrderedFloat(self.stroke_width).hash(state);
        self.stroke_cap.hash(state);
        self.stroke_join.hash(state);
        if let Some(stroke_dash) = &self.stroke_dash {
            stroke_dash.iter().for_each(|d| OrderedFloat(*d).hash(state));
        } else {
            OrderedFloat(0.0).hash(state);
        }
        self.zindex.hash(state);
    }
}

impl SceneLineMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, None)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, None)
    }

    pub fn defined_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.defined.as_iter(self.len as usize, None)
    }

    pub fn transformed_path(&self, origin: [f32; 2]) -> Path {
        let mut defined_paths: Vec<Path> = Vec::new();

        // Build path for each defined line segment
        let mut path_builder = Path::builder().with_svg();
        let mut path_len = 0;
        for (x, y, defined) in itertools::izip!(self.x_iter(), self.y_iter(), self.defined_iter()) {
            if *defined {
                if path_len > 0 {
                    // Continue path
                    path_builder.line_to(point(*x + origin[0], *y + origin[1]));
                } else {
                    // New path
                    path_builder.move_to(point(*x + origin[0], *y + origin[1]));
                }
                path_len += 1;
            } else {
                if path_len == 1 {
                    // Finishing single point line. Add extra point at the same location
                    // so that stroke caps are drawn
                    path_builder.close();
                }
                defined_paths.push(path_builder.build());
                path_builder = Path::builder().with_svg();
                path_len = 0;
            }
        }
        defined_paths.push(path_builder.build());

        if let Some(stroke_dash) = &self.stroke_dash {
            // Create new paths with dashing
            let mut dash_path_builder = Path::builder();

            // Process each defined path
            for path in &defined_paths {
                let path_measurements = PathMeasurements::from_path(path, 0.1);
                let mut sampler =
                    PathSampler::new(&path_measurements, path, &(), SampleType::Distance);

                // Next index into stroke_dash array
                let mut dash_idx = 0;

                // Distance along line from (x0,y0) to (x1,y1) where the next dash will start
                let mut start_dash_dist: f32 = 0.0;

                // Total length of line
                let line_len = sampler.length();

                // Whether the next dash length represents a drawn dash (draw == true)
                // or a gap (draw == false)
                let mut draw = true;

                while start_dash_dist < line_len {
                    let end_dash_dist = if start_dash_dist + stroke_dash[dash_idx] >= line_len {
                        // The final dash/gap should be truncated to the end of the line
                        line_len
                    } else {
                        // The dash/gap fits entirely in the rule
                        start_dash_dist + stroke_dash[dash_idx]
                    };

                    if draw {
                        sampler.split_range(start_dash_dist..end_dash_dist, &mut dash_path_builder);
                    }

                    // update start dist for next dash/gap
                    start_dash_dist = end_dash_dist;

                    // increment index and cycle back to start of start of dash array
                    dash_idx = (dash_idx + 1) % stroke_dash.len();

                    // Alternate between drawn dash and gap
                    draw = !draw;
                }
            }
            dash_path_builder.build()
        } else {
            // Combine all defined paths into one
            let mut combined_builder = Path::builder();
            for path in defined_paths {
                for event in path.iter() {
                    combined_builder.path_event(event);
                }
            }
            combined_builder.build()
        }
    }
}

impl Default for SceneLineMark {
    fn default() -> Self {
        Self {
            name: "line_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            defined: ScalarOrArray::new_scalar(true),
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            stroke_width: 1.0,
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            stroke_dash: None,
            zindex: None,
        }
    }
}

impl From<SceneLineMark> for SceneMark {
    fn from(mark: SceneLineMark) -> Self {
        SceneMark::Line(mark)
    }
}
