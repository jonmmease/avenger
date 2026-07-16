use std::{cmp::Ordering, fmt::Write};

use serde::{Deserialize, Serialize};

use crate::{SourceFile, SourceMap, SourceSpan};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagnosticCode(String);

impl DiagnosticCode {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLabel {
    pub span: SourceSpan,
    pub message: String,
}

impl SourceLabel {
    pub fn new(span: SourceSpan, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionOrImportFrame {
    pub span: SourceSpan,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub primary: SourceLabel,
    pub secondary: Vec<SourceLabel>,
    pub notes: Vec<String>,
    pub trace: Vec<ExpansionOrImportFrame>,
}

impl Diagnostic {
    pub fn error(
        code: impl Into<String>,
        message: impl Into<String>,
        primary: SourceLabel,
    ) -> Self {
        Self {
            code: DiagnosticCode::new(code),
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            primary,
            secondary: Vec::new(),
            notes: Vec::new(),
            trace: Vec::new(),
        }
    }

    pub fn with_secondary(mut self, label: SourceLabel) -> Self {
        self.secondary.push(label);
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// Sort diagnostics independently of discovery order. Source display names are
/// used before opaque IDs so the result remains stable when loading is parallel.
pub fn sort_diagnostics(diagnostics: &mut [Diagnostic], sources: &SourceMap) {
    diagnostics.sort_by(|left, right| compare_diagnostic(left, right, sources));
}

fn compare_diagnostic(left: &Diagnostic, right: &Diagnostic, sources: &SourceMap) -> Ordering {
    source_name(left, sources)
        .cmp(&source_name(right, sources))
        .then_with(|| left.primary.span.range.cmp(&right.primary.span.range))
        .then_with(|| left.code.cmp(&right.code))
        .then_with(|| left.message.cmp(&right.message))
}

fn source_name(diagnostic: &Diagnostic, sources: &SourceMap) -> String {
    sources
        .get(diagnostic.primary.span.source)
        .map(|source| source.origin.display_name())
        .unwrap_or_else(|| diagnostic.primary.span.source.to_string())
}

pub fn render_diagnostics(diagnostics: &[Diagnostic], sources: &SourceMap) -> String {
    let mut diagnostics = diagnostics.to_vec();
    sort_diagnostics(&mut diagnostics, sources);
    let mut output = String::new();
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        let severity = match diagnostic.severity {
            DiagnosticSeverity::Error => "error",
        };
        writeln!(
            output,
            "{severity}[{}]: {}",
            diagnostic.code.as_str(),
            diagnostic.message
        )
        .expect("write to string");
        render_label(&mut output, &diagnostic.primary, sources, true);

        let mut secondary = diagnostic.secondary.clone();
        secondary.sort_by_key(|label| label.span);
        for label in &secondary {
            render_label(&mut output, label, sources, false);
        }
        for note in &diagnostic.notes {
            writeln!(output, "  = note: {note}").expect("write to string");
        }
        for frame in &diagnostic.trace {
            let origin = sources
                .get(frame.span.source)
                .map(|source| source.origin.display_name())
                .unwrap_or_else(|| frame.span.source.to_string());
            writeln!(output, "  = via {origin}: {}", frame.message).expect("write to string");
        }
    }
    output
}

fn render_label(output: &mut String, label: &SourceLabel, sources: &SourceMap, primary: bool) {
    let Some(source) = sources.get(label.span.source) else {
        writeln!(
            output,
            " --> {}:{}..{}: {}",
            label.span.source, label.span.range.start, label.span.range.end, label.message
        )
        .expect("write to string");
        return;
    };
    let Ok(location) = source.line_index().location(label.span.range.start) else {
        writeln!(
            output,
            " --> {}:{}..{}: {}",
            source.origin, label.span.range.start, label.span.range.end, label.message
        )
        .expect("write to string");
        return;
    };
    let marker = if primary { "-->" } else { ":::" };
    writeln!(
        output,
        " {marker} {}:{}:{}",
        source.origin,
        location.line + 1,
        location.column + 1
    )
    .expect("write to string");
    render_excerpt(
        output,
        source,
        label,
        location.line,
        location.display_column,
    );
}

fn render_excerpt(
    output: &mut String,
    source: &SourceFile,
    label: &SourceLabel,
    line: usize,
    display_column: usize,
) {
    let text = source.line_index().line_text(line).unwrap_or("");
    let line_number = line + 1;
    writeln!(output, " {line_number:>3} | {text}").expect("write to string");

    let end = label.span.range.end.max(label.span.range.start + 1);
    let width = source
        .line_index()
        .location(end.min(source.text().len()))
        .ok()
        .filter(|end_location| end_location.line == line)
        .map(|end_location| end_location.display_column.saturating_sub(display_column))
        .unwrap_or(1)
        .max(1);
    writeln!(
        output,
        "     | {}{} {}",
        " ".repeat(display_column),
        "^".repeat(width),
        label.message
    )
    .expect("write to string");
}
