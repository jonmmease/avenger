/// A parsed D3 datetime pattern. Locale directives expand during preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern(pub(crate) Vec<PatternToken>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PatternToken {
    Literal(String),
    Directive { code: char, padding: Option<char> },
}
