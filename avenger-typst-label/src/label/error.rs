use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum LabelInitError {
    #[error("requested Typst backend is unavailable: {0}")]
    BackendUnavailable(&'static str),
}

#[derive(Debug, Error, PartialEq)]
pub enum LabelError {
    #[error("none of the requested font families is available: {family}")]
    MissingFont { family: String },

    #[error("source is {actual} bytes, exceeding max_source_bytes={limit}")]
    SourceTooLarge { actual: usize, limit: usize },

    #[error("math span count is {actual}, exceeding max_math_spans={limit}")]
    TooManyMathSpans { actual: usize, limit: usize },

    #[error("math nesting depth is {actual}, exceeding max_math_depth={limit}")]
    MathDepthExceeded { actual: usize, limit: usize },

    #[error("empty math fragment at byte range {start}..{end}")]
    EmptyMathFragment { start: usize, end: usize },

    #[error("unsupported Typst label syntax at byte {position}: {message}")]
    UnsupportedSyntax {
        position: usize,
        message: &'static str,
    },

    #[error("unsupported Typst label feature {feature:?} at byte {position}: {message}")]
    UnsupportedFeature {
        position: usize,
        feature: String,
        message: &'static str,
    },

    #[error("label parameter name {name:?} collides with retained Typst {namespace} name")]
    ParameterNameCollision {
        name: String,
        namespace: &'static str,
    },

    #[error("Typst syntax error at byte {position}: {message}")]
    Syntax { position: usize, message: String },

    #[error("requested output is not supported yet: {0}")]
    UnsupportedOutput(&'static str),

    #[error("engine error in byte range {start}..{end}: {message}")]
    Engine {
        start: usize,
        end: usize,
        message: String,
    },
}
