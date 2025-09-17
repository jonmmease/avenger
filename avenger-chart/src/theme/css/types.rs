//! Core type definitions for CSS theme system

use std::collections::{HashMap, HashSet};

/// Simple RGBA color representation
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RGBA {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl RGBA {
    pub fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

/// CSS-based theme for chart styling
#[derive(Clone, Debug)]
pub struct Theme {
    /// CSS rules sorted by source order
    pub rules: Vec<Rule>,
    /// CSS custom properties (variables)
    pub variables: HashMap<String, ThemeValue>,
    /// Set of properties that inherit by default
    pub inherited_properties: HashSet<String>,
    /// Optional source map for debugging
    pub source_map: Option<SourceMap>,
    /// Base font size for rem calculations (default: 12.0)
    pub base_font_size: f32,
    /// Chart dimensions for viewport units (set at render time)
    pub chart_width: Option<f32>,
    pub chart_height: Option<f32>,
}

/// A CSS rule with selector and declarations
#[derive(Clone, Debug)]
pub struct Rule {
    /// The CSS selector
    pub selector: String, // Store as string
    /// Parsed selector (not used in simple implementation)
    pub parsed_selector: Option<()>,
    /// CSS declarations
    pub declarations: Vec<Declaration>,
    /// Specificity of the selector
    pub specificity: Specificity,
    /// Source line number (for debugging)
    pub source_line: Option<usize>,
}

/// A CSS declaration (property-value pair)
#[derive(Clone, Debug)]
pub struct Declaration {
    /// Property name (case-insensitive)
    pub property: String,
    /// Property value
    pub value: ThemeValue,
    /// Whether this is !important
    pub important: bool,
}

/// Standard CSS specificity (inline, id, class, element)
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Copy)]
pub struct Specificity(pub u32, pub u32, pub u32, pub u32);

/// Theme property value
#[derive(Clone, Debug, PartialEq)]
pub enum ThemeValue {
    /// String value
    String(String),
    /// Numeric value
    Number(f64),
    /// Color value
    Color(RGBA),
    /// Dimension with unit
    Dimension(f64, Unit),
    /// List of values
    List(Vec<ThemeValue>),
    /// Keyword value
    Keyword(String),
    /// CSS variable reference with optional fallback
    Variable(String, Option<Box<ThemeValue>>),
    /// calc() expression (stored as string, evaluated when needed)
    Calc(String),
    /// No value
    None,
}

/// CSS units
#[derive(Clone, Debug, PartialEq, Copy)]
pub enum Unit {
    /// Pixels
    Px,
    /// Root em (relative to base font size)
    Rem,
    /// Em (relative to current font size)
    Em,
    /// Percentage
    Percent,
    /// Viewport width (1% of chart width)
    Vw,
    /// Viewport height (1% of chart height)
    Vh,
    /// Unitless number
    None,
}

/// Theme error types
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("CSS parse error: {0}")]
    ParseError(String),

    #[error("Invalid selector: {0}")]
    InvalidSelector(String),

    #[error("Invalid value for property {property}: {value}")]
    InvalidValue { property: String, value: String },

    #[error("Import error: {0}")]
    ImportError(String),

    #[error("Circular import: {0}")]
    CircularImport(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Source map for debugging
#[derive(Clone, Debug)]
pub struct SourceMap {
    /// Rule locations in source
    pub rules: Vec<RuleLocation>,
}

/// Location of a rule in source
#[derive(Clone, Debug)]
pub struct RuleLocation {
    /// Index of the rule
    pub rule_index: usize,
    /// Start line number
    pub start_line: usize,
    /// End line number
    pub end_line: usize,
    /// Source file name (if from a file)
    pub source_file: Option<String>,
}

/// Context for theme property queries
#[derive(Clone, Debug)]
pub struct ThemeContext {
    /// Element type (e.g., "mark", "axis", "legend", "title")
    pub element_type: String,
    /// Coordinate system type (e.g., "cartesian", "polar")
    pub coord_type: Option<String>,
    /// Mark type (e.g., "symbol", "line", "rect")
    pub mark_type: Option<String>,
    /// Scale type (e.g., "linear", "ordinal")
    pub scale_type: Option<String>,

    /// Element ID
    pub id: Option<String>,
    /// CSS classes
    pub classes: Vec<String>,
    /// HTML-style attributes
    pub attributes: HashMap<String, String>,

    /// Element path in hierarchy
    pub path: Vec<String>,
    /// Parent context
    pub parent: Option<Box<ThemeContext>>,

    /// Pseudo-class states
    pub states: HashSet<String>,
    /// Position among siblings (0-based)
    pub index: Option<usize>,
    /// Total number of siblings
    pub total_siblings: Option<usize>,
}

impl ThemeContext {
    /// Create a new context for an element type
    pub fn new(element_type: impl Into<String>) -> Self {
        Self {
            element_type: element_type.into(),
            coord_type: None,
            mark_type: None,
            scale_type: None,
            id: None,
            classes: Vec::new(),
            attributes: HashMap::new(),
            path: Vec::new(),
            parent: None,
            states: HashSet::new(),
            index: None,
            total_siblings: None,
        }
    }

    /// Add a CSS class
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.classes.push(class.into());
        self
    }

    /// Set the mark type
    pub fn with_mark(mut self, mark: impl Into<String>) -> Self {
        self.mark_type = Some(mark.into());
        self
    }

    /// Set the coordinate system type
    pub fn with_coord(mut self, coord: impl Into<String>) -> Self {
        self.coord_type = Some(coord.into());
        self
    }

    /// Set the scale type
    pub fn with_scale(mut self, scale: impl Into<String>) -> Self {
        self.scale_type = Some(scale.into());
        self
    }

    /// Set the element ID
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Add an attribute
    pub fn with_attr(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Add a pseudo-class state
    pub fn with_state(mut self, state: impl Into<String>) -> Self {
        self.states.insert(state.into());
        self
    }

    /// Set the parent context
    pub fn with_parent(mut self, parent: ThemeContext) -> Self {
        self.parent = Some(Box::new(parent));
        self
    }
}

/// Property explanation for debugging
#[derive(Debug)]
pub struct PropertyExplanation {
    /// Final computed value
    pub final_value: ThemeValue,
    /// All rules that matched the context
    pub matching_rules: Vec<Rule>,
    /// The rule that won the cascade
    pub winning_rule: Option<Rule>,
    /// Specificity order of matching rules
    pub specificity_order: Vec<Specificity>,
}

/// Theme usage report for debugging
#[derive(Debug)]
pub struct ThemeUsageReport {
    /// Rules that were never matched
    pub unused_rules: Vec<Rule>,
    /// Properties that were queried
    pub queried_properties: HashSet<String>,
    /// Contexts that were evaluated
    pub evaluated_contexts: Vec<ThemeContext>,
}

/// Usage tracker for theme debugging
#[derive(Debug, Default)]
pub struct UsageTracker {
    /// Set of rule indices that have been matched
    pub matched_rules: HashSet<usize>,
    /// Properties that have been queried
    pub queried_properties: HashSet<String>,
    /// Contexts that have been evaluated
    pub evaluated_contexts: Vec<ThemeContext>,
}

impl UsageTracker {
    /// Create a new usage tracker
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a rule match
    pub fn record_match(&mut self, rule_index: usize) {
        self.matched_rules.insert(rule_index);
    }

    /// Record a property query
    pub fn record_query(&mut self, property: &str) {
        self.queried_properties.insert(property.to_lowercase());
    }

    /// Record a context evaluation
    pub fn record_context(&mut self, context: ThemeContext) {
        self.evaluated_contexts.push(context);
    }

    /// Generate a usage report
    pub fn generate_report(&self, theme: &Theme) -> ThemeUsageReport {
        let unused_rules = theme
            .rules
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.matched_rules.contains(i))
            .map(|(_, r)| r.clone())
            .collect();

        ThemeUsageReport {
            unused_rules,
            queried_properties: self.queried_properties.clone(),
            evaluated_contexts: self.evaluated_contexts.clone(),
        }
    }
}

// Property name constants
pub mod props {
    // Typography
    pub const FONT_FAMILY: &str = "font-family";
    pub const FONT_SIZE: &str = "font-size";
    pub const FONT_WEIGHT: &str = "font-weight";
    pub const FONT_STYLE: &str = "font-style";
    pub const LINE_HEIGHT: &str = "line-height";
    pub const LETTER_SPACING: &str = "letter-spacing";
    pub const TEXT_ALIGN: &str = "text-align";

    // Colors
    pub const COLOR: &str = "color";
    pub const BACKGROUND_COLOR: &str = "background-color";
    pub const FILL: &str = "fill";
    pub const STROKE: &str = "stroke";

    // Dimensions
    pub const WIDTH: &str = "width";
    pub const HEIGHT: &str = "height";

    // Padding (Taffy layout)
    pub const PADDING: &str = "padding";
    pub const PADDING_TOP: &str = "padding-top";
    pub const PADDING_RIGHT: &str = "padding-right";
    pub const PADDING_BOTTOM: &str = "padding-bottom";
    pub const PADDING_LEFT: &str = "padding-left";

    // Margin (Taffy layout)
    pub const MARGIN: &str = "margin";
    pub const MARGIN_TOP: &str = "margin-top";
    pub const MARGIN_RIGHT: &str = "margin-right";
    pub const MARGIN_BOTTOM: &str = "margin-bottom";
    pub const MARGIN_LEFT: &str = "margin-left";

    // Flexbox/Grid (Taffy layout)
    pub const DISPLAY: &str = "display";
    pub const FLEX_DIRECTION: &str = "flex-direction";
    pub const JUSTIFY_CONTENT: &str = "justify-content";
    pub const ALIGN_ITEMS: &str = "align-items";
    pub const ALIGN_SELF: &str = "align-self";
    pub const ALIGN_CONTENT: &str = "align-content";
    pub const GAP: &str = "gap";
    pub const ROW_GAP: &str = "row-gap";
    pub const COLUMN_GAP: &str = "column-gap";
    pub const FLEX_GROW: &str = "flex-grow";
    pub const FLEX_SHRINK: &str = "flex-shrink";
    pub const FLEX_BASIS: &str = "flex-basis";

    // Chart-specific
    pub const LABEL_ANGLE: &str = "label-angle";
    pub const GRID_COLOR: &str = "grid-color";
    pub const GRID_OPACITY: &str = "grid-opacity";
    pub const TICK_LENGTH: &str = "tick-length";
    pub const TICK_COLOR: &str = "tick-color";
    pub const DOMAIN_COLOR: &str = "domain-color";
    pub const STROKE_WIDTH: &str = "stroke-width";
    pub const STROKE_DASH: &str = "stroke-dash";
    pub const SIZE: &str = "size";
    pub const OPACITY: &str = "opacity";
    pub const SHAPE: &str = "shape";
}
