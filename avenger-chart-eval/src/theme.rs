//! The theme of an evaluator: the defaults of every style that a chart leaves unset.

use std::collections::BTreeMap;

use avenger_common::types::FontWeight;
use avenger_guides::{
    attribution::AttributionConfig,
    axis::AxisStyle,
    facet::FacetGuideStyle,
    legend::{ColorbarConfig, LegendStyle},
};
use avenger_scenegraph::marks::text::TextHalo;

/// The defaults of every style that a chart leaves unset, with the defaults of Vega-Lite and the
/// bundled Lato fonts. An evaluator takes a theme, so one chart draws with the theme of each host.
#[derive(Clone, Debug)]
pub struct Theme {
    /// The color of a mark without a color channel.
    pub mark_color: [f32; 4],
    /// The area of a symbol without a size channel, in square pixels.
    pub symbol_size: f32,
    /// The font size of a text mark without a size channel.
    pub font_size: f32,
    /// The font family of text marks, a CSS family list.
    pub font_family: String,
    /// The stroke width of lines, trails, and rules without a stroke width channel.
    pub stroke_width: f32,
    /// The width of a trail without a size channel.
    pub trail_size: f32,
    /// The thickness of a bar or a rect at a continuous key, without a second position.
    pub bar_thickness: f32,
    /// The width and the height of an image mark that does not set them.
    pub image_size: [f32; 2],
    /// The share of a band scale's step between its bands, when its spec sets no
    /// `padding_inner`.
    pub band_padding_inner: f32,
    /// The share of a band scale's step before its first band and after its last, when its spec
    /// sets no `padding_outer`.
    pub band_padding_outer: f32,
    /// The share of a point scale's step before its first point and after its last, when its spec
    /// sets no `padding`.
    pub point_padding: f32,
    /// Named color palettes, each a list of CSS colors.
    pub palettes: BTreeMap<String, Vec<String>>,
    /// The palette of a discrete color scale without a range.
    pub category_palette: String,
    /// The palette of a continuous color scale without a range, whose colors it interpolates.
    pub sequential_palette: String,
    /// The shapes of a discrete shape scale without a range.
    pub shapes: Vec<String>,
    /// The stroke dashes of a discrete stroke dash scale without a range.
    pub stroke_dashes: Vec<Vec<f32>>,
    /// The patterns of a discrete pattern scale without a range.
    pub patterns: Vec<String>,
    /// The range of a size scale of symbols.
    pub size_range: [f32; 2],
    /// The range of a size scale of text, the font sizes.
    pub font_size_range: [f32; 2],
    /// The range of a size scale of trails, their widths.
    pub trail_size_range: [f32; 2],
    /// The range of an opacity scale.
    pub opacity_range: [f32; 2],
    /// The range of a stroke width scale.
    pub stroke_width_range: [f32; 2],
    /// The range of an angle scale, in degrees.
    pub angle_range: [f32; 2],
    /// The bins of a quantize or a quantile scale without a range.
    pub bins: usize,
    /// The pattern of the numbers of continuous axes, colorbars, and legends that set no
    /// format, in the syntax of the number formatter of the text engine.
    pub number_format: String,
    /// The style of axes.
    pub axis: AxisStyle,
    /// Whether a continuous axis, and an axis of a polar plot, draws grid lines.
    pub grid: bool,
    /// The halo behind the labels of the axes of polar plots, which stand over their grid lines
    /// and their data, or `None` for none.
    pub polar_label_halo: Option<TextHalo>,
    /// The style of legends.
    pub legend: LegendStyle,
    /// The configuration of colorbars.
    pub colorbar: ColorbarConfig,
    /// The longest that a colorbar runs along its side when its spec sets no longest length.
    pub colorbar_length: Option<f32>,
    /// The style of titles.
    pub title: TitleStyle,
    /// The configuration of attributions.
    pub attribution: AttributionConfig,
    /// The space between a plot's guides and its legends, in pixels.
    pub legend_offset: f32,
    /// The space between two legends of a side, in pixels.
    pub legend_gap: f32,
    /// The space between the children of a row or a column that sets no gap, in pixels, which
    /// grows to hold the guides between them.
    pub concat_gap: f32,
    /// The narrowest width of a column of a wrap that sets none, in pixels.
    pub wrap_min_column: f32,
    /// The style of the headers of facets.
    pub facet: FacetGuideStyle,
    /// The space between the labels of a facet header and the header of a facet inside it on its
    /// side, in pixels, larger than the space beside a plot, so the levels of a nested facet read
    /// apart.
    pub facet_nested_label_padding: f32,
    /// The style of the graticules, the spheres, and the geoshapes of map plots.
    pub map: MapStyle,
    /// The space around the chart, in pixels.
    pub padding: f32,
}

/// The style of the parts of map plots that a chart leaves unset.
#[derive(Clone, Debug)]
pub struct MapStyle {
    /// The color of the lines of a graticule.
    pub graticule_stroke: [f32; 4],
    /// The width of the lines of a graticule, in pixels.
    pub graticule_stroke_width: f32,
    /// The font size of the labels of a graticule, in the theme's font family.
    pub graticule_label_font_size: f32,
    pub graticule_label_font_weight: FontWeight,
    pub graticule_label_color: [f32; 4],
    /// The space between the end of a graticule line and its label, in pixels.
    pub graticule_label_padding: f32,
    /// The fill of the sphere behind the data.
    pub sphere_fill: [f32; 4],
    /// The outline of the sphere.
    pub sphere_stroke: [f32; 4],
    pub sphere_stroke_width: f32,
    /// The stroke of the geometries of a geoshape mark without a stroke channel.
    pub geoshape_stroke: [f32; 4],
    /// The width of the stroke of a geoshape mark that sets none, in pixels.
    pub geoshape_stroke_width: f32,
}

/// The style of a title and its subtitle.
#[derive(Clone, Debug)]
pub struct TitleStyle {
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub color: [f32; 4],
    pub subtitle_font_size: f32,
    pub subtitle_font_weight: FontWeight,
    pub subtitle_color: [f32; 4],
    /// The space between the title and its subtitle, in pixels.
    pub subtitle_gap: f32,
    /// The space between a title and what it titles, in pixels.
    pub offset: f32,
    /// Where a title that sets no alignment stands along its side.
    pub align: TitleAlign,
}

/// Where a title stands along its side, in the direction that it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleAlign {
    Start,
    Center,
    End,
}

impl Default for Theme {
    fn default() -> Self {
        let palettes = [
            (
                "tableau10",
                &[
                    "#4c78a8", "#f58518", "#e45756", "#72b7b2", "#54a24b", "#eeca3b", "#b279a2",
                    "#ff9da6", "#9d755d", "#bab0ac",
                ][..],
            ),
            (
                "category10",
                &[
                    "#1f77b4", "#ff7f0e", "#2ca02c", "#d62728", "#9467bd", "#8c564b", "#e377c2",
                    "#7f7f7f", "#bcbd22", "#17becf",
                ][..],
            ),
            (
                "blues",
                &["#cfe1f2", "#93c4de", "#4a98c9", "#1764ab", "#08306b"][..],
            ),
            (
                "viridis",
                &["#440154", "#3b528b", "#21918c", "#5ec962", "#fde725"][..],
            ),
            (
                "redblue",
                &[
                    "#b2182b", "#ef8a62", "#fddbc7", "#d1e5f0", "#67a9cf", "#2166ac",
                ][..],
            ),
        ]
        .into_iter()
        .map(|(name, colors)| {
            (
                name.to_string(),
                colors.iter().map(|color| color.to_string()).collect(),
            )
        })
        .collect();
        let gray = [0.4, 0.4, 0.4, 1.0];
        Self {
            mark_color: [0.298, 0.471, 0.659, 1.0],
            symbol_size: 30.0,
            font_size: 11.0,
            font_family: "sans-serif".to_string(),
            stroke_width: 2.0,
            trail_size: 2.0,
            bar_thickness: 5.0,
            image_size: [50.0, 50.0],
            band_padding_inner: 0.1,
            band_padding_outer: 0.05,
            point_padding: 0.5,
            palettes,
            category_palette: "tableau10".to_string(),
            sequential_palette: "blues".to_string(),
            shapes: [
                "circle",
                "square",
                "cross",
                "diamond",
                "triangle-up",
                "triangle-down",
                "triangle-right",
                "triangle-left",
            ]
            .iter()
            .map(|shape| shape.to_string())
            .collect(),
            stroke_dashes: vec![
                vec![],
                vec![4.0, 2.0],
                vec![2.0, 1.0],
                vec![1.0, 1.0],
                vec![1.0, 2.0, 4.0, 2.0],
            ],
            patterns: [
                "diagonal-down",
                "dots",
                "crosshatch",
                "horizontal",
                "rings",
                "vertical",
                "triangles",
                "checkerboard",
            ]
            .iter()
            .map(|pattern| pattern.to_string())
            .collect(),
            size_range: [9.0, 361.0],
            font_size_range: [8.0, 40.0],
            trail_size_range: [1.0, 10.0],
            opacity_range: [0.3, 0.8],
            stroke_width_range: [1.0, 4.0],
            angle_range: [0.0, 360.0],
            bins: 5,
            number_format: ",f".to_string(),
            axis: AxisStyle {
                label_font_size: 10.0,
                title_font_size: 11.0,
                title_font_weight: FontWeight::BOLD,
                tick_color: [0.533, 0.533, 0.533, 1.0],
                domain_color: [0.533, 0.533, 0.533, 1.0],
                ..AxisStyle::default()
            },
            grid: true,
            polar_label_halo: Some(TextHalo {
                color: [1.0, 1.0, 1.0, 0.75].into(),
                width: 2.0,
            }),
            legend: LegendStyle {
                title_font_weight: FontWeight::BOLD,
                ..LegendStyle::default()
            },
            colorbar: ColorbarConfig::default(),
            colorbar_length: Some(200.0),
            title: TitleStyle {
                font_family: "sans-serif".to_string(),
                font_size: 13.0,
                font_weight: FontWeight::BOLD,
                color: [0.0, 0.0, 0.0, 1.0],
                subtitle_font_size: 11.0,
                subtitle_font_weight: FontWeight::NORMAL,
                subtitle_color: gray,
                subtitle_gap: 2.0,
                offset: 10.0,
                align: TitleAlign::Start,
            },
            attribution: AttributionConfig::default(),
            legend_offset: 12.0,
            legend_gap: 12.0,
            concat_gap: 20.0,
            wrap_min_column: 160.0,
            facet: FacetGuideStyle {
                label_color: [0.35, 0.35, 0.35, 1.0],
                label_font_size: 11.0,
                label_font_family: "sans-serif".to_string(),
                title_font_family: "sans-serif".to_string(),
                title_font_weight: FontWeight::BOLD,
                ..FacetGuideStyle::default()
            },
            facet_nested_label_padding: 14.0,
            map: MapStyle {
                graticule_stroke: [0.0, 0.0, 0.0, 0.15],
                graticule_stroke_width: 0.7,
                graticule_label_font_size: 10.0,
                graticule_label_font_weight: FontWeight::NORMAL,
                graticule_label_color: gray,
                graticule_label_padding: 4.0,
                sphere_fill: [0.94, 0.97, 1.0, 1.0],
                sphere_stroke: [0.533, 0.533, 0.533, 1.0],
                sphere_stroke_width: 1.0,
                geoshape_stroke: [1.0, 1.0, 1.0, 1.0],
                geoshape_stroke_width: 0.5,
            },
            padding: 5.0,
        }
    }
}

impl Theme {
    /// The colors of the palette `name`, or `None` for a palette that the theme does not name.
    pub fn palette(&self, name: &str) -> Option<&[String]> {
        self.palettes.get(name).map(Vec::as_slice)
    }
}
