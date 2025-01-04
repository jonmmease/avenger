// use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
// use avenger_guides::legend::symbol::{make_symbol_legend, SymbolLegendConfig};
// use avenger_scenegraph::marks::{mark::SceneMark, symbol::SymbolShape};

// use crate::types::scales::Scale;

// use super::Guide;

// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
// pub enum LegendDirection {
//     Horizontal,
//     Vertical,
// }

// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
// pub enum LegendOrient {
//     Top,
//     Bottom,
//     Left,
//     Right,
// }

// #[derive(Debug, Clone)]
// pub struct SymbolLegend {
//     pub text_scale: Option<Scale>,
//     pub shape_scale: Option<Scale>,
//     pub size_scale: Option<Scale>,
//     pub stroke_scale: Option<Scale>,
//     pub fill_scale: Option<Scale>,
//     pub angle_scale: Option<Scale>,
// }

// impl SymbolLegend {
//     pub fn new() -> Self {
//         Self {
//             config: SymbolLegendConfig::default(),
//         }
//     }

//     pub fn title(self, title: String) -> Self {
//         Self {
//             config: SymbolLegendConfig {
//                 title: Some(title),
//                 ..self.config
//             },
//         }
//     }

//     pub fn get_title(&self) -> Option<&str> {
//         self.config.title.as_deref()
//     }

//     pub fn text<T: Into<ScalarOrArray<String>>>(self, text: T) -> Self {
//         Self {
//             config: SymbolLegendConfig {
//                 text: text.into(),
//                 ..self.config
//             },
//         }
//     }

//     pub fn get_text(&self) -> &ScalarOrArray<String> {
//         &self.config.text
//     }

//     pub fn shape<T: Into<ScalarOrArray<SymbolShape>>>(self, shape: T) -> Self {
//         Self {
//             config: SymbolLegendConfig {
//                 shape: shape.into(),
//                 ..self.config
//             },
//         }
//     }

//     pub fn get_shape(&self) -> &ScalarOrArray<SymbolShape> {
//         &self.config.shape
//     }

//     pub fn size<T: Into<ScalarOrArray<f32>>>(self, size: T) -> Self {
//         Self {
//             config: SymbolLegendConfig {
//                 size: size.into(),
//                 ..self.config
//             },
//         }
//     }

//     pub fn get_size(&self) -> &ScalarOrArray<f32> {
//         &self.config.size
//     }

//     pub fn stroke<T: Into<ScalarOrArray<ColorOrGradient>>>(self, stroke: T) -> Self {
//         Self {
//             config: SymbolLegendConfig {
//                 stroke: stroke.into(),
//                 ..self.config
//             },
//         }
//     }

//     pub fn get_stroke(&self) -> &ScalarOrArray<ColorOrGradient> {
//         &self.config.stroke
//     }

//     pub fn stroke_width(self, stroke_width: f32) -> Self {
//         Self {
//             config: SymbolLegendConfig {
//                 stroke_width: Some(stroke_width),
//                 ..self.config
//             },
//         }
//     }

//     pub fn get_stroke_width(&self) -> Option<f32> {
//         self.config.stroke_width
//     }

//     pub fn fill<T: Into<ScalarOrArray<ColorOrGradient>>>(self, fill: T) -> Self {
//         Self {
//             config: SymbolLegendConfig {
//                 fill: fill.into(),
//                 ..self.config
//             },
//         }
//     }

//     pub fn get_fill(&self) -> &ScalarOrArray<ColorOrGradient> {
//         &self.config.fill
//     }

//     pub fn angle<T: Into<ScalarOrArray<f32>>>(self, angle: T) -> Self {
//         Self {
//             config: SymbolLegendConfig {
//                 angle: angle.into(),
//                 ..self.config
//             },
//         }
//     }

//     pub fn get_angle(&self) -> &ScalarOrArray<f32> {
//         &self.config.angle
//     }
// }

// impl Guide for SymbolLegend {
//     fn compile(
//         &self,
//         context: &super::GuideCompilationContext,
//     ) -> Result<Vec<SceneMark>, crate::error::AvengerChartError> {
//         let legend_group = make_symbol_legend(&self.config)?;
//         Ok(vec![legend_group.into()])
//     }
// }
