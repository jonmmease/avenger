use crate::marks::arc::ArcMark;
use crate::marks::area::AreaMark;
use crate::marks::line::LineMark;
use crate::marks::path::PathMark;
use crate::marks::rect::RectMark;
use crate::marks::rule::RuleMark;
use crate::marks::symbol::SymbolMark;
use crate::marks::text::TextMark;
use crate::marks::util::{decode_color, decode_gradient};
use avenger::marks::group::{Clip, SceneGroup as RsSceneGroup};
use avenger::marks::mark::SceneMark;
use lyon_path::builder::BorderRadii;
use lyon_path::geom::euclid::Point2D;
use lyon_path::geom::Box2D;
use lyon_path::Winding;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct GroupMark {
    inner: RsSceneGroup,
}

impl GroupMark {
    pub fn build(self) -> RsSceneGroup {
        self.inner
    }
}

#[wasm_bindgen]
impl GroupMark {
    #[wasm_bindgen(constructor)]
    pub fn new(
        origin_x: f32,
        origin_y: f32,
        name: Option<String>,
        width: Option<f32>,
        height: Option<f32>,
    ) -> Self {
        let clip = if let (Some(width), Some(height)) = (width, height) {
            Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: width.clone(),
                height: height.clone(),
            }
        } else {
            Clip::None
        };

        Self {
            inner: RsSceneGroup {
                origin: [origin_x, origin_y],
                name: name.unwrap_or_default(),
                clip,
                ..Default::default()
            },
        }
    }

    pub fn set_clip(
        &mut self,
        width: Option<f32>,
        height: Option<f32>,
        corner_radius: Option<f32>,
        corner_radius_top_left: Option<f32>,
        corner_radius_top_right: Option<f32>,
        corner_radius_bottom_left: Option<f32>,
        corner_radius_bottom_right: Option<f32>,
    ) {
        let clip = if let (Some(width), Some(height)) = (width, height) {
            let corner_radius = corner_radius.unwrap_or(0.0);
            let corner_radius_top_left = corner_radius_top_left.unwrap_or(corner_radius);
            let corner_radius_top_right = corner_radius_top_right.unwrap_or(corner_radius);
            let corner_radius_bottom_left = corner_radius_bottom_left.unwrap_or(corner_radius);
            let corner_radius_bottom_right = corner_radius_bottom_right.unwrap_or(corner_radius);

            if corner_radius_top_left > 0.0
                || corner_radius_top_right > 0.0
                || corner_radius_bottom_left > 0.0
                || corner_radius_bottom_right > 0.0
            {
                // Rounded rectangle path
                let mut builder = lyon_path::Path::builder();
                builder.add_rounded_rectangle(
                    &Box2D::new(Point2D::new(0.0, 0.0), Point2D::new(width, height)),
                    &BorderRadii {
                        top_left: corner_radius_top_left,
                        top_right: corner_radius_top_right,
                        bottom_left: corner_radius_bottom_left,
                        bottom_right: corner_radius_bottom_right,
                    },
                    Winding::Positive,
                );
                Clip::Path(builder.build())
            } else {
                // Rect
                Clip::Rect {
                    x: 0.0, // x and y are zero to align with origin
                    y: 0.0,
                    width,
                    height,
                }
            }
        } else {
            Clip::None
        };
        self.inner.clip = clip;
    }

    /// Set fill color
    ///
    /// @param {string} color_value
    /// @param {number} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_fill(&mut self, color_value: &str, opacity: f32) -> Result<(), JsError> {
        self.inner.fill = Some(decode_color(color_value, opacity)?);
        Ok(())
    }

    /// Set fill gradient
    ///
    /// @param {(string|object)} value
    /// @param {number} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_fill_gradient(&mut self, value: JsValue, opacity: f32) -> Result<(), JsError> {
        let grad = decode_gradient(value, opacity, &mut self.inner.gradients)?;
        self.inner.fill = Some(grad);
        Ok(())
    }

    /// Set stroke color
    ///
    /// @param {string} color_value
    /// @param {number} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke(&mut self, color_value: &str, opacity: f32) -> Result<(), JsError> {
        self.inner.stroke = Some(decode_color(color_value, opacity)?);
        Ok(())
    }

    /// Set stroke gradient
    ///
    /// @param {(string|object)} value
    /// @param {number} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke_gradient(&mut self, value: JsValue, opacity: f32) -> Result<(), JsError> {
        let grad = decode_gradient(value, opacity, &mut self.inner.gradients)?;
        self.inner.stroke = Some(grad);
        Ok(())
    }

    pub fn set_stroke_width(&mut self, width: Option<f32>) {
        self.inner.stroke_width = width;
    }

    pub fn add_symbol_mark(&mut self, mark: SymbolMark) {
        self.inner.marks.push(SceneMark::Symbol(mark.build()));
    }

    pub fn add_rect_mark(&mut self, mark: RectMark) {
        self.inner.marks.push(SceneMark::Rect(mark.build()));
    }

    pub fn add_rule_mark(&mut self, mark: RuleMark) {
        self.inner.marks.push(SceneMark::Rule(mark.build()));
    }

    pub fn add_text_mark(&mut self, mark: TextMark) {
        self.inner
            .marks
            .push(SceneMark::Text(Box::new(mark.build())));
    }

    pub fn add_arc_mark(&mut self, mark: ArcMark) {
        self.inner.marks.push(SceneMark::Arc(mark.build()));
    }

    pub fn add_path_mark(&mut self, mark: PathMark) {
        self.inner.marks.push(SceneMark::Path(mark.build()));
    }

    pub fn add_line_mark(&mut self, mark: LineMark) {
        self.inner.marks.push(SceneMark::Line(mark.build()));
    }

    pub fn add_area_mark(&mut self, mark: AreaMark) {
        self.inner.marks.push(SceneMark::Area(mark.build()));
    }

    pub fn add_group_mark(&mut self, mark: GroupMark) {
        self.inner.marks.push(SceneMark::Group(mark.inner));
    }
}
