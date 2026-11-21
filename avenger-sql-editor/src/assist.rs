//! The editor's assistance outside egui: the state of the completion popup, which items it shows
//! as the user types, and the text after an insert, the fixes of the errors at the cursor, the
//! definition and the references of the name at the cursor, and the blocks of a hover.
//! Positions are character indices, as egui's cursor counts them, and the requests of the
//! `Document` take byte offsets, which [`Chars`] converts.

use std::ops::Range;

use avenger_sql_language::{CompletionItem, CompletionKind, Document, Fix};
use avenger_sql_syntax::FunctionRole;

use crate::highlight::Chars;

/// The most items that the popup lists at once.
pub const SHOWN: usize = 8;

/// An open completion popup: the start of the text that its items replace, the number of the
/// characters after the cursor that they replace too, its items, and the selected item among
/// those that the typed text keeps.
#[derive(Clone, Debug)]
pub struct Popup {
    start: usize,
    tail: usize,
    items: Vec<CompletionItem>,
    selected: usize,
}

/// The quotes that open a quoted name or a string, which the popup's filter skips.
const QUOTES: [char; 3] = ['`', '\'', '"'];

impl Popup {
    /// The popup of `document`'s completions at the character `cursor`, or `None` without items.
    /// An item replaces the whole name or string at the cursor, through its end.
    pub fn open(document: &Document, cursor: usize) -> Option<Self> {
        let chars = Chars::new(document.text());
        let offset = chars.offset(cursor) as u32;
        let completions = document.completions(offset.into());
        if completions.items.is_empty() {
            return None;
        }
        Some(Self {
            start: chars.range(completions.range).start,
            tail: chars.range(completions.replace).end.saturating_sub(cursor),
            items: completions.items,
            selected: 0,
        })
    }

    /// The text from the start of the popup to `cursor`, which filters its items, or `None` when
    /// the cursor left the name that the popup completes.
    fn typed(&self, text: &str, cursor: usize) -> Option<String> {
        if cursor < self.start {
            return None;
        }
        let typed: String = text
            .chars()
            .skip(self.start)
            .take(cursor - self.start)
            .collect();
        let name = typed.trim_start_matches(QUOTES);
        name.chars()
            .all(|c| c.is_alphanumeric() || c == '_')
            .then(|| name.to_ascii_lowercase())
    }

    /// The items that `text`, typed up to `cursor`, keeps: those whose labels start with the
    /// typed name, without regard to case or an opening quote, in the order of the completion.
    pub fn items(&self, text: &str, cursor: usize) -> Vec<&CompletionItem> {
        let Some(typed) = self.typed(text, cursor) else {
            return Vec::new();
        };
        (self.items.iter())
            .filter(|item| {
                let label = item.label.trim_start_matches(QUOTES);
                label.to_ascii_lowercase().starts_with(&typed)
            })
            .collect()
    }

    /// Whether the popup closes at `cursor` of `text`: the cursor left its name, or no item stays.
    pub fn closes(&self, text: &str, cursor: usize) -> bool {
        self.items(text, cursor).is_empty()
    }

    /// The index of the selected item among the kept items of `text` at `cursor`.
    pub fn selected(&self, text: &str, cursor: usize) -> usize {
        self.selected
            .min(self.items(text, cursor).len().saturating_sub(1))
    }

    /// Moves the selection by `step` among the kept items, and stops at the first and the last.
    pub fn select(&mut self, text: &str, cursor: usize, step: isize) {
        let count = self.items(text, cursor).len();
        let selected = self.selected(text, cursor) as isize + step;
        self.selected = selected.clamp(0, count.saturating_sub(1) as isize) as usize;
    }

    /// Selects the kept item at `index`.
    pub fn select_at(&mut self, index: usize) {
        self.selected = index;
    }

    /// `text` with the selected item inserted in place of the text from the popup's start
    /// through the end of the name or string at `cursor`, and the character index after the
    /// insert.
    pub fn accept(&self, text: &str, cursor: usize) -> Option<(String, usize)> {
        let items = self.items(text, cursor);
        let item = items.get(self.selected(text, cursor))?;
        let chars = Chars::new(text);
        let (start, end) = (chars.offset(self.start), chars.offset(cursor + self.tail));
        let mut accepted = text.to_string();
        accepted.replace_range(start..end, &item.insert);
        Some((accepted, self.start + item.insert.chars().count()))
    }
}

/// The heading of the items of `kind` in the completion popup, which lists the items of each kind
/// together.
pub fn heading(kind: CompletionKind) -> &'static str {
    match kind {
        CompletionKind::Keyword => "Keywords",
        CompletionKind::Column => "Columns",
        CompletionKind::RangeVariable => "Range variables",
        CompletionKind::Variable => "Variables",
        CompletionKind::Field => "Fields",
        CompletionKind::Table => "Tables",
        CompletionKind::Query => "Queries",
        CompletionKind::Parameter => "Parameters",
        CompletionKind::EnumValue => "Values",
        CompletionKind::Type => "Types",
        CompletionKind::Window => "Windows",
        CompletionKind::Option => "Options",
        CompletionKind::File => "Files",
        CompletionKind::Folder => "Folders",
        CompletionKind::Function(role) => match role {
            FunctionRole::Scalar | FunctionRole::Unresolved => "Functions",
            FunctionRole::Aggregate => "Aggregate functions",
            FunctionRole::Window => "Window functions",
            FunctionRole::Table => "Table functions",
            FunctionRole::Mark => "Marks",
            FunctionRole::Constructor => "Constructors",
            FunctionRole::Transform => "Transforms",
            FunctionRole::Composition => "Compositions",
        },
    }
}

/// The details of a completion item, beside the popup while it is selected: its documentation,
/// or else its detail, such as a column's type. Empty for an item without either.
pub fn details(item: &CompletionItem) -> Vec<Block> {
    match (&item.documentation, &item.detail) {
        (Some(documentation), _) => blocks(documentation),
        (None, Some(detail)) => vec![
            Block::Code(item.label.clone()),
            Block::Paragraph(spans(detail)),
        ],
        (None, None) => Vec::new(),
    }
}

/// The fixes of the errors of `document` whose ranges hold the character `cursor`.
pub fn fixes(document: &Document, cursor: usize) -> Vec<Fix> {
    let chars = Chars::new(document.text());
    (document.diagnostics().into_iter())
        .filter(|diagnostic| {
            let range = chars.range(diagnostic.range);
            range.start <= cursor && cursor <= range.end
        })
        .flat_map(|diagnostic| diagnostic.fixes)
        .collect()
}

/// `text` with the edits of `fix` applied, last first, so that each edit's range is a range of
/// the text before the edits.
pub fn apply(text: &str, fix: &Fix) -> String {
    let mut edits = fix.edits.clone();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start()));
    let mut applied = text.to_string();
    for edit in edits {
        let range = usize::from(edit.range.start())..usize::from(edit.range.end());
        applied.replace_range(range, &edit.replacement);
    }
    applied
}

/// The range of characters of the definition of the name at the character `cursor`.
pub fn definition(document: &Document, cursor: usize) -> Option<Range<usize>> {
    let chars = Chars::new(document.text());
    let link = document.definition((chars.offset(cursor) as u32).into())?;
    Some(chars.range(link.target))
}

/// The ranges of characters of the definition of the name at the character `cursor` and of each
/// use of it, in the order of the text.
pub fn references(document: &Document, cursor: usize) -> Vec<Range<usize>> {
    let chars = Chars::new(document.text());
    (document
        .references((chars.offset(cursor) as u32).into())
        .into_iter())
    .map(|reference| chars.range(reference.range))
    .collect()
}

/// A run of the text of a block of Markdown, with its style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Plain(String),
    /// A span in backticks.
    Code(String),
    /// A span in double asterisks.
    Strong(String),
}

/// The blocks of the Markdown of a hover or of a parameter's documentation, which the language
/// crate writes: headings, paragraphs, items of lists, and fenced blocks of code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(Vec<Span>),
    Paragraph(Vec<Span>),
    Item(Vec<Span>),
    Code(String),
}

/// The open paragraph or item of [`blocks`], as its kind and its joined lines.
type Open = Option<(fn(Vec<Span>) -> Block, String)>;

/// The blocks of `markdown`. The lines of a paragraph or an item join with spaces, and a fence
/// may be indented, as in the item of a list, whose indent its lines lose.
pub fn blocks(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut open: Open = None;
    let mut fence: Option<(usize, Vec<String>)> = None;
    let close = |open: &mut Open, blocks: &mut Vec<Block>| {
        if let Some((kind, text)) = open.take() {
            blocks.push(kind(spans(&text)));
        }
    };
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some((fenced, lines)) = &mut fence {
            if trimmed.starts_with("```") {
                blocks.push(Block::Code(lines.join("\n")));
                fence = None;
            } else {
                let cut = (line.char_indices())
                    .take_while(|(index, c)| *index < *fenced && *c == ' ')
                    .count();
                lines.push(line[cut..].to_string());
            }
            continue;
        }
        if trimmed.starts_with("```") {
            close(&mut open, &mut blocks);
            fence = Some((indent, Vec::new()));
        } else if trimmed.is_empty() {
            close(&mut open, &mut blocks);
        } else if let Some(heading) = trimmed.strip_prefix('#') {
            close(&mut open, &mut blocks);
            blocks.push(Block::Heading(spans(
                heading.trim_start_matches('#').trim(),
            )));
        } else if let Some(item) = ["- ", "+ ", "* "]
            .iter()
            .find_map(|marker| trimmed.strip_prefix(marker))
        {
            close(&mut open, &mut blocks);
            open = Some((Block::Item, item.trim().to_string()));
        } else if let Some((_, text)) = &mut open {
            text.push(' ');
            text.push_str(trimmed.trim_end());
        } else {
            open = Some((Block::Paragraph, trimmed.trim_end().to_string()));
        }
    }
    close(&mut open, &mut blocks);
    if let Some((_, lines)) = fence {
        blocks.push(Block::Code(lines.join("\n")));
    }
    blocks
}

/// The spans of a line of Markdown: code in backticks, and else strong text in double asterisks.
fn spans(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for (index, part) in text.split('`').enumerate() {
        if index % 2 == 1 {
            spans.push(Span::Code(part.to_string()));
            continue;
        }
        for (index, part) in part.split("**").enumerate() {
            if part.is_empty() {
                continue;
            }
            spans.push(match index % 2 {
                1 => Span::Strong(part.to_string()),
                _ => Span::Plain(part.to_string()),
            });
        }
    }
    spans
}
