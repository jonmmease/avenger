//! Shape sequences for categorical shape encoding

/// Shape sequence configuration
#[derive(Clone, Debug)]
pub struct ShapeSequence {
    /// Ordered list of shape names for categorical encoding
    pub shapes: Vec<&'static str>,
}

impl Default for ShapeSequence {
    fn default() -> Self {
        Self {
            shapes: vec![
                "circle",
                "cross",
                "diamond",
                "square",
                "star",
                "triangle-up",
                "wye",
                "cushion",
            ],
        }
    }
}

impl ShapeSequence {
    /// Create a new shape sequence
    pub fn new(shapes: Vec<&'static str>) -> Self {
        Self { shapes }
    }

    /// Get shapes limited to a specific count
    pub fn get_shapes(&self, count: Option<usize>) -> Vec<&'static str> {
        match count {
            Some(n) if n <= self.shapes.len() => self.shapes[..n].to_vec(),
            _ => self.shapes.clone(),
        }
    }

    /// Get shapes as string values
    pub fn get_shape_strings(&self, count: Option<usize>) -> Vec<String> {
        self.get_shapes(count)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Get all shape names as strings
    pub fn get_shape_names(&self) -> Vec<String> {
        self.shapes.iter().map(|s| s.to_string()).collect()
    }
}
