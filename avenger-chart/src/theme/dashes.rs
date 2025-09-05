//! Dash patterns for line styling

/// Dash pattern configuration
#[derive(Clone, Debug)]
pub struct DashPatterns {
    /// Named dash patterns
    pub patterns: Vec<&'static str>,
}

impl Default for DashPatterns {
    fn default() -> Self {
        Self {
            patterns: vec![
                "solid",
                "dashed",
                "dotted",
                "long-dash",
                "dash-dot",
                "long-short",
                "even-short",
                "double-dash",
            ],
        }
    }
}

impl DashPatterns {
    /// Create a new dash pattern sequence
    pub fn new(patterns: Vec<&'static str>) -> Self {
        Self { patterns }
    }

    /// Get patterns limited to a specific count
    pub fn get_patterns(&self, count: Option<usize>) -> Vec<&'static str> {
        match count {
            Some(n) if n <= self.patterns.len() => self.patterns[..n].to_vec(),
            _ => self.patterns.clone(),
        }
    }

    /// Get patterns as string values
    pub fn get_pattern_strings(&self, count: Option<usize>) -> Vec<String> {
        self.get_patterns(count)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }
    
    /// Get all dash pattern names as strings
    pub fn get_dash_names(&self) -> Vec<String> {
        self.patterns.iter().map(|s| s.to_string()).collect()
    }
}