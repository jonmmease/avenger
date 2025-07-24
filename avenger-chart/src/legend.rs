/// Legend configuration for visualizations
#[derive(Clone, Debug)]
pub struct Legend {
    pub visible: bool,
    pub title: Option<String>,
    pub position: Option<LegendPosition>,
    pub orientation: Option<LegendOrientation>,
    pub symbol_size: Option<f64>,
    pub gradient_length: Option<f64>,
    pub gradient_thickness: Option<f64>,
    pub columns: Option<usize>,
    pub label_limit: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LegendPosition {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LegendOrientation {
    Horizontal,
    Vertical,
}

impl Legend {
    pub fn new() -> Self {
        Self {
            visible: true,
            position: None,
            title: None,
            orientation: None,
            symbol_size: None,
            gradient_length: None,
            gradient_thickness: None,
            columns: None,
            label_limit: None,
        }
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.orientation = Some(orientation);
        self
    }

    pub fn symbol_size(mut self, size: f64) -> Self {
        self.symbol_size = Some(size);
        self
    }

    pub fn gradient_length(mut self, length: f64) -> Self {
        self.gradient_length = Some(length);
        self
    }

    pub fn gradient_thickness(mut self, thickness: f64) -> Self {
        self.gradient_thickness = Some(thickness);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = Some(columns);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.label_limit = Some(limit);
        self
    }
}

impl Default for Legend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::coords::Cartesian;
    use crate::legend::{LegendOrientation, LegendPosition};
    use crate::plot::Plot;

    #[test]
    fn test_legend_fill_with_configuration() {
        let plot = Plot::new(Cartesian)
            .scale_fill(|scale| scale)
            .legend_fill(|legend| legend.title("Temperature").position(LegendPosition::Right));

        // Should have legend configured
        assert!(plot.legends.contains_key("fill"));
        let legend = &plot.legends["fill"];
        assert_eq!(legend.title, Some("Temperature".to_string()));
        assert_eq!(legend.position, Some(LegendPosition::Right));
    }

    #[test]
    fn test_legend_fill_with_visible_false() {
        let plot = Plot::new(Cartesian)
            .scale_fill(|scale| scale)
            .legend_fill(|legend| legend.visible(false));

        // Legend exists but is marked invisible
        assert!(plot.legends.contains_key("fill"));
        let legend = &plot.legends["fill"];
        assert!(!legend.visible);
    }

    #[test]
    fn test_legend_stroke_with_orientation() {
        let plot = Plot::new(Cartesian)
            .scale_stroke(|scale| scale)
            .legend_stroke(|legend| {
                legend
                    .title("Category")
                    .orientation(LegendOrientation::Horizontal)
                    .columns(3)
            });

        assert!(plot.legends.contains_key("stroke"));
        let legend = &plot.legends["stroke"];
        assert_eq!(legend.title, Some("Category".to_string()));
        assert_eq!(legend.orientation, Some(LegendOrientation::Horizontal));
        assert_eq!(legend.columns, Some(3));
    }

    #[test]
    fn test_legend_size_with_symbol_size() {
        let plot = Plot::new(Cartesian)
            .scale_size(|scale| scale)
            .legend_size(|legend| legend.title("Population").symbol_size(20.0));

        assert!(plot.legends.contains_key("size"));
        let legend = &plot.legends["size"];
        assert_eq!(legend.title, Some("Population".to_string()));
        assert_eq!(legend.symbol_size, Some(20.0));
    }

    #[test]
    fn test_legend_opacity_with_gradient() {
        let plot = Plot::new(Cartesian)
            .scale_opacity(|scale| scale)
            .legend_opacity(|legend| {
                legend
                    .title("Confidence")
                    .gradient_length(150.0)
                    .gradient_thickness(15.0)
            });

        assert!(plot.legends.contains_key("opacity"));
        let legend = &plot.legends["opacity"];
        assert_eq!(legend.title, Some("Confidence".to_string()));
        assert_eq!(legend.gradient_length, Some(150.0));
        assert_eq!(legend.gradient_thickness, Some(15.0));
    }

    #[test]
    fn test_multiple_legends() {
        let plot = Plot::new(Cartesian)
            .scale_fill(|scale| scale)
            .scale_size(|scale| scale)
            .legend_fill(|legend| legend.title("Temperature").position(LegendPosition::Right))
            .legend_size(|legend| legend.title("Population").position(LegendPosition::Left));

        assert_eq!(plot.legends.len(), 2);
        assert!(plot.legends.contains_key("fill"));
        assert!(plot.legends.contains_key("size"));
    }

    #[test]
    fn test_legend_modification() {
        let plot = Plot::new(Cartesian)
            .scale_fill(|scale| scale)
            .legend_fill(|legend| legend.title("First Title"))
            .legend_fill(|legend| legend.title("Updated Title"));

        // Second call should update the existing legend
        assert_eq!(plot.legends.len(), 1);
        let legend = &plot.legends["fill"];
        assert_eq!(legend.title, Some("Updated Title".to_string()));
    }
}
