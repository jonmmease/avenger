use avenger_color::ColorOrGradient;
use avenger_common::types::{StrokeCap, StrokeJoin};
use avenger_scenegraph::{
    marks::{
        area::SceneAreaMark, group::Clip, line::SceneLineMark, mark::SceneMark,
        path::ScenePathMark, rect::SceneRectMark, rule::SceneRuleMark,
    },
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use itertools::izip;

use crate::{
    error::AvengerSvgError,
    options::{SvgBackground, SvgRenderOptions},
    path::{format_number, lyon_path_to_svg_d, push_number, push_point},
    style::{push_fill_attrs, push_stroke_attrs},
};

#[derive(Debug, Clone, Default)]
pub struct SvgRenderer {
    options: SvgRenderOptions,
}

impl SvgRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(mut self, options: SvgRenderOptions) -> Self {
        self.options = options;
        self
    }

    pub fn render_scene_graph(&self, scene_graph: &SceneGraph) -> Result<String, AvengerSvgError> {
        let mut output = String::new();
        let precision = self.options.precision;
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);

        output.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" width=""#);
        output.push_str(&format_number(scene_graph.width, precision)?);
        output.push_str(r#"" height=""#);
        output.push_str(&format_number(scene_graph.height, precision)?);
        output.push_str(r#"" viewBox="0 0 "#);
        output.push_str(&format_number(scene_graph.width, precision)?);
        output.push(' ');
        output.push_str(&format_number(scene_graph.height, precision)?);
        output.push_str("\">\n");
        output.push_str("<defs/>\n");
        output.push_str("<g fill=\"none\" stroke-miterlimit=\"10\">\n");

        self.write_background(&mut output)?;

        for item in display_list.ordered_items() {
            if item.clip != Clip::None {
                return Err(AvengerSvgError::UnsupportedFeature(
                    "clip paths are not implemented in avenger-svg yet".to_string(),
                ));
            }

            match &item.mark {
                SceneDisplayMark::OwnedGroupPath(mark) => {
                    self.write_path_mark(&mut output, mark, item.origin)?;
                }
                SceneDisplayMark::Borrowed(mark) => {
                    self.write_scene_mark(&mut output, mark, item.origin)?;
                }
            }
        }

        output.push_str("</g>\n</svg>\n");
        Ok(output)
    }

    fn write_background(&self, output: &mut String) -> Result<(), AvengerSvgError> {
        let color = match self.options.background {
            SvgBackground::White => Some([1.0, 1.0, 1.0, 1.0]),
            SvgBackground::Transparent => None,
            SvgBackground::Color(color) => Some(color),
        };

        let Some(color) = color else {
            return Ok(());
        };

        output.push_str(r#"<rect width="100%" height="100%""#);
        push_fill_attrs(
            output,
            Some(&ColorOrGradient::Color(color)),
            self.options.precision,
        )?;
        output.push_str(r#" stroke="none"/>"#);
        output.push('\n');
        Ok(())
    }

    fn write_scene_mark(
        &self,
        output: &mut String,
        mark: &SceneMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerSvgError> {
        match mark {
            SceneMark::Rect(mark) => self.write_rect_mark(output, mark, origin),
            SceneMark::Path(mark) => self.write_path_mark(output, mark, origin),
            SceneMark::Rule(mark) => self.write_rule_mark(output, mark, origin),
            SceneMark::Line(mark) => self.write_line_mark(output, mark, origin),
            SceneMark::Area(mark) => self.write_area_mark(output, mark, origin),
            SceneMark::Arc(_) => Err(AvengerSvgError::UnsupportedMark("arc")),
            SceneMark::Symbol(_) => Err(AvengerSvgError::UnsupportedMark("symbol")),
            SceneMark::Trail(_) => Err(AvengerSvgError::UnsupportedMark("trail")),
            SceneMark::Text(_) => Err(AvengerSvgError::UnsupportedMark("text")),
            SceneMark::Image(_) => Err(AvengerSvgError::UnsupportedMark("image")),
            SceneMark::Group(_) => Ok(()),
        }
    }

    fn write_rect_mark(
        &self,
        output: &mut String,
        mark: &SceneRectMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, stroke, stroke_width) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter()
        ) {
            self.write_path_element(
                output,
                &lyon_path_to_svg_d(&path, self.options.precision)?,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                },
            )?;
        }

        Ok(())
    }

    fn write_path_mark(
        &self,
        output: &mut String,
        mark: &ScenePathMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, stroke) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter()
        ) {
            self.write_path_element(
                output,
                &lyon_path_to_svg_d(&path, self.options.precision)?,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: mark.stroke_width,
                    stroke_cap: Some(mark.stroke_cap),
                    stroke_join: Some(mark.stroke_join),
                    stroke_dash: None,
                },
            )?;
        }

        Ok(())
    }

    fn write_area_mark(
        &self,
        output: &mut String,
        mark: &SceneAreaMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerSvgError> {
        let path = mark.transformed_path(origin);
        self.write_path_element(
            output,
            &lyon_path_to_svg_d(&path, self.options.precision)?,
            PathStyle {
                fill: Some(&mark.fill),
                stroke: Some(&mark.stroke),
                stroke_width: Some(mark.stroke_width),
                stroke_cap: Some(mark.stroke_cap),
                stroke_join: Some(mark.stroke_join),
                stroke_dash: mark.stroke_dash.as_deref(),
            },
        )
    }

    fn write_line_mark(
        &self,
        output: &mut String,
        mark: &SceneLineMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerSvgError> {
        let d = line_path_d(mark, origin, self.options.precision)?;
        self.write_path_element(
            output,
            &d,
            PathStyle {
                fill: None,
                stroke: Some(&mark.stroke),
                stroke_width: Some(mark.stroke_width),
                stroke_cap: Some(mark.stroke_cap),
                stroke_join: Some(mark.stroke_join),
                stroke_dash: mark.stroke_dash.as_deref(),
            },
        )
    }

    fn write_rule_mark(
        &self,
        output: &mut String,
        mark: &SceneRuleMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerSvgError> {
        let stroke_dashes = mark
            .stroke_dash_iter()
            .map(|iter| iter.collect::<Vec<_>>())
            .unwrap_or_default();

        for (index, (x1, y1, x2, y2, stroke, stroke_width, stroke_cap)) in izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.x2_iter(),
            mark.y2_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
            mark.stroke_cap_iter(),
        )
        .enumerate()
        {
            output.push_str("<line x1=\"");
            push_number(output, *x1 + origin[0], self.options.precision)?;
            output.push_str("\" y1=\"");
            push_number(output, *y1 + origin[1], self.options.precision)?;
            output.push_str("\" x2=\"");
            push_number(output, *x2 + origin[0], self.options.precision)?;
            output.push_str("\" y2=\"");
            push_number(output, *y2 + origin[1], self.options.precision)?;
            output.push('"');
            push_fill_attrs(output, None, self.options.precision)?;
            push_stroke_attrs(
                output,
                Some(stroke),
                Some(*stroke_width),
                Some(*stroke_cap),
                None,
                stroke_dashes.get(index).map(|dash| dash.as_slice()),
                self.options.precision,
            )?;
            output.push_str("/>\n");
        }

        Ok(())
    }

    fn write_path_element(
        &self,
        output: &mut String,
        d: &str,
        style: PathStyle<'_>,
    ) -> Result<(), AvengerSvgError> {
        output.push_str(r#"<path d=""#);
        output.push_str(d);
        output.push('"');
        push_fill_attrs(output, style.fill, self.options.precision)?;
        push_stroke_attrs(
            output,
            style.stroke,
            style.stroke_width,
            style.stroke_cap,
            style.stroke_join,
            style.stroke_dash,
            self.options.precision,
        )?;
        output.push_str("/>\n");
        Ok(())
    }
}

struct PathStyle<'a> {
    fill: Option<&'a ColorOrGradient>,
    stroke: Option<&'a ColorOrGradient>,
    stroke_width: Option<f32>,
    stroke_cap: Option<StrokeCap>,
    stroke_join: Option<StrokeJoin>,
    stroke_dash: Option<&'a [f32]>,
}

fn line_path_d(
    mark: &SceneLineMark,
    origin: [f32; 2],
    precision: usize,
) -> Result<String, AvengerSvgError> {
    let mut d = String::new();
    let mut path_len = 0;
    let mut last = None;

    for (x, y, defined) in izip!(mark.x_iter(), mark.y_iter(), mark.defined_iter()) {
        if *defined {
            let point = [*x + origin[0], *y + origin[1]];
            if path_len == 0 {
                if !d.is_empty() {
                    d.push(' ');
                }
                d.push('M');
            } else {
                d.push(' ');
                d.push('L');
            }
            push_point(&mut d, point[0], point[1], precision)?;
            path_len += 1;
            last = Some(point);
        } else {
            close_single_point_subpath(&mut d, path_len, last, precision)?;
            path_len = 0;
            last = None;
        }
    }

    close_single_point_subpath(&mut d, path_len, last, precision)?;
    Ok(d)
}

fn close_single_point_subpath(
    d: &mut String,
    path_len: usize,
    last: Option<[f32; 2]>,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    if path_len == 1 {
        if let Some([x, y]) = last {
            d.push(' ');
            d.push('L');
            push_point(d, x, y, precision)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use avenger_color::ColorOrGradient;
    use avenger_common::value::ScalarOrArray;
    use avenger_scenegraph::{
        marks::{group::SceneGroup, rect::SceneRectMark, rule::SceneRuleMark},
        scene_graph::SceneGraph,
    };

    use super::*;

    #[test]
    fn renders_simple_rect_svg_that_usvg_can_parse() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                width: Some(ScalarOrArray::new_scalar(4.0)),
                height: Some(ScalarOrArray::new_scalar(5.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg""#));
        assert!(svg.contains(r##"fill="#ff0000""##));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn renders_rule_with_native_line_and_dasharray() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [1.0, 2.0],
            marks: vec![SceneRuleMark {
                len: 1,
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                x2: ScalarOrArray::new_scalar(4.0),
                y2: ScalarOrArray::new_scalar(5.0),
                stroke_dash: Some(ScalarOrArray::new_scalar(vec![2.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r#"<line x1="3" y1="5" x2="5" y2="7""#));
        assert!(svg.contains(r#"stroke-dasharray="2 1""#));
    }

    #[test]
    fn renders_group_fill_path_from_display_list() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                clip: avenger_scenegraph::marks::group::Clip::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                fill: Some(ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r##"fill="#0000ff""##));
    }
}
