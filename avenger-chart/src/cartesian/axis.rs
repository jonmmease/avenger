use crate::axis::Axis;
use crate::theme::CssTheme;
use crate::error::AvengerChartError;
use crate::maybe::Maybe;
use avenger_scenegraph::marks::mark::SceneMark;
use serde::{Deserialize, Serialize};
use std::any::Any;

/// Position for Cartesian axes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AxisPosition {
    Top,
    Right,
    Bottom,
    Left,
}

/// Concrete struct for Cartesian axes
/// Using a struct instead of a trait enables type inference in closure parameters
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CartesianAxis {
    pub visible: Maybe<bool>,
    pub position: Maybe<AxisPosition>,
    pub title: Maybe<Option<String>>,
    pub grid: Maybe<bool>,
    pub tick_count: Maybe<Option<usize>>,
    pub label_angle: Maybe<f32>,
    pub format_number: Maybe<Option<String>>,
    pub title_font_family: Maybe<Option<String>>,
    pub label_font_family: Maybe<Option<String>>,
}

impl CartesianAxis {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = Maybe::Set(visible);
        self
    }

    pub fn position(mut self, position: AxisPosition) -> Self {
        self.position = Maybe::Set(position);
        self
    }

    pub fn title<S: Into<String>>(mut self, title: S) -> Self {
        self.title = Maybe::Set(Some(title.into()));
        self
    }

    pub fn grid(mut self, grid: bool) -> Self {
        self.grid = Maybe::Set(grid);
        self
    }

    pub fn tick_count(mut self, count: usize) -> Self {
        self.tick_count = Maybe::Set(Some(count));
        self
    }

    pub fn label_angle(mut self, angle: f32) -> Self {
        self.label_angle = Maybe::Set(angle);
        self
    }

    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format_number = Maybe::Set(Some(format.into()));
        self
    }

    pub fn title_font_family(mut self, font: impl Into<String>) -> Self {
        self.title_font_family = Maybe::Set(Some(font.into()));
        self
    }

    pub fn label_font_family(mut self, font: impl Into<String>) -> Self {
        self.label_font_family = Maybe::Set(Some(font.into()));
        self
    }

    /// Update this axis configuration with another, applying all set fields
    pub fn update(mut self, other: CartesianAxis) -> Self {
        if other.visible.is_set() {
            self.visible = other.visible;
        }
        if other.position.is_set() {
            self.position = other.position;
        }
        if other.title.is_set() {
            self.title = other.title;
        }
        if other.grid.is_set() {
            self.grid = other.grid;
        }
        if other.tick_count.is_set() {
            self.tick_count = other.tick_count;
        }
        if other.label_angle.is_set() {
            self.label_angle = other.label_angle;
        }
        if other.format_number.is_set() {
            self.format_number = other.format_number;
        }
        if other.title_font_family.is_set() {
            self.title_font_family = other.title_font_family;
        }
        if other.label_font_family.is_set() {
            self.label_font_family = other.label_font_family;
        }
        self
    }

    /// Render this axis to scene marks
    pub fn render(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &crate::layout::LayoutBounds,
        theme: &CssTheme,
    ) -> Result<SceneMark, AvengerChartError> {
        use avenger_guides::axis::{
            band::make_band_axis_marks,
            numeric::make_numeric_axis_marks,
            opts::{AxisConfig, AxisOrientation},
            point::make_point_axis_marks,
        };

        // Skip if invisible (default to visible if not set)
        if !self.visible.clone().unwrap_or(true) {
            return Ok(SceneMark::Group(
                avenger_scenegraph::marks::group::SceneGroup {
                    marks: vec![],
                    ..Default::default()
                },
            ));
        }

        // Determine axis position
        let position = self.position.clone().unwrap_or_else(|| {
            // Default positions based on channel name
            match channel {
                "x" => AxisPosition::Bottom,
                "y" => AxisPosition::Left,
                _ => AxisPosition::Bottom,
            }
        });

        // Convert position to orientation
        let orientation = match position {
            AxisPosition::Top => AxisOrientation::Top,
            AxisPosition::Bottom => AxisOrientation::Bottom,
            AxisPosition::Left => AxisOrientation::Left,
            AxisPosition::Right => AxisOrientation::Right,
        };

        // Axis origin is always the top-left corner of the plot area
        let axis_origin = [plot_bounds.x, plot_bounds.y];

        // Use coordinate system type and channel for CSS selector support
        // e.g., guide[type="cartesian"] axis[type="x"]
        let coord_type = Some("cartesian");
        let axis_type = Some(channel);

        // Always use theme colors for axis elements
        let label_color = crate::utils::parse_color_to_array(&theme.axis_label_color(coord_type, axis_type));
        let title_color = crate::utils::parse_color_to_array(&theme.axis_title_color(coord_type, axis_type));
        let domain_color = crate::utils::parse_color_to_array(&theme.axis_domain_color(coord_type, axis_type));
        let tick_color = crate::utils::parse_color_to_array(&theme.axis_tick_color(coord_type, axis_type));

        // Create axis config with plot dimensions and theme
        let axis_config = AxisConfig {
            orientation,
            dimensions: [plot_width, plot_height],
            grid: self.grid.clone().unwrap_or(false),
            format_number: self.format_number.clone().flatten(),
            title_font_size: Some(theme.axis_title_font_size(coord_type, axis_type)),
            // Pass colors (potentially overridden for dark backgrounds)
            domain_color: Some(domain_color),
            tick_color: Some(tick_color),
            grid_color: Some({
                let mut color = crate::utils::parse_color_to_array(&theme.axis_grid_color(coord_type, axis_type));
                color[3] = theme.axis_grid_opacity(coord_type, axis_type); // Apply opacity to alpha channel
                color
            }),
            grid_width: Some(theme.axis_grid_width(coord_type, axis_type)),
            label_color: Some(label_color),
            title_color: Some(title_color),
            tick_length: Some(theme.axis_tick_length(coord_type, axis_type)),
            label_font_size: Some(theme.axis_label_font_size(coord_type, axis_type)),
            label_font_weight: Some(theme.axis_label_font_weight(coord_type, axis_type)),
            title_font_weight: Some(theme.axis_title_font_weight(coord_type, axis_type)),
            label_font_family: Some(
                self.label_font_family
                    .clone()
                    .flatten()
                    .unwrap_or_else(|| theme.axis_label_font_family(coord_type, axis_type)),
            ),
            title_font_family: Some(
                self.title_font_family
                    .clone()
                    .flatten()
                    .unwrap_or_else(|| theme.axis_title_font_family(coord_type, axis_type)),
            ),
        };

        // Generate axis marks based on scale characteristics
        // Use domain and range kinds to determine which axis maker to use
        use avenger_scales::scales::DomainKind;

        let domain_kind = scale.scale_impl.domain_kind();
        let _range_kind = scale.scale_impl.range_kind();

        // For categorical domains, check the scale type
        let axis_group = match domain_kind {
            DomainKind::Categorical => {
                // Use scale_type to distinguish band/point/ordinal
                let scale_type = scale.scale_impl.scale_type();
                match scale_type {
                    "band" => make_band_axis_marks(
                        scale,
                        self.title.clone().flatten().as_deref().unwrap_or(""),
                        axis_origin,
                        &axis_config,
                    )?,
                    "point" => make_point_axis_marks(
                        scale.clone(),
                        self.title.clone().flatten().as_deref().unwrap_or(""),
                        axis_origin,
                        &axis_config,
                    )?,
                    "ordinal" => {
                        // Ordinal scales with discrete ranges need band-like rendering
                        // Convert to band scale for axis rendering
                        use avenger_scales::scales::band::BandScale;
                        let band_scale = BandScale::from_point_scale(scale);
                        make_band_axis_marks(
                            &band_scale,
                            self.title.clone().flatten().as_deref().unwrap_or(""),
                            axis_origin,
                            &axis_config,
                        )?
                    }
                    _ => {
                        return Err(AvengerChartError::InternalError(format!(
                            "Unsupported scale type '{}' for categorical domain on axis '{}'",
                            scale_type, channel
                        )));
                    }
                }
            }
            _ => {
                // All continuous domain scales use numeric axis
                make_numeric_axis_marks(
                    scale,
                    self.title.clone().flatten().as_deref().unwrap_or(""),
                    axis_origin,
                    &axis_config,
                )?
            }
        };

        Ok(SceneMark::Group(axis_group))
    }
}

#[typetag::serde]
impl Axis for CartesianAxis {
    fn update(&mut self, other: &dyn Axis) {
        if let Some(o) = other.as_any().downcast_ref::<CartesianAxis>() {
            *self = self.clone().update(o.clone());
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn box_clone(&self) -> Box<dyn Axis> {
        Box::new(self.clone())
    }
}
