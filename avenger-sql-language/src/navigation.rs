//! Go to definition, references, and rename, from the definition of each name that the analysis
//! resolved.

use avenger_sql::analysis::{Definition, Model};
use avenger_sql_syntax::{SyntaxKind, SyntaxNode, TextEdit, TextRange};

use crate::{node_at, quote, Document, Offset};

/// A name and the range that defines it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefinitionLink {
    /// The range of the name, which an editor underlines as the link.
    pub origin: TextRange,
    /// The range that defines the name.
    pub target: TextRange,
}

/// A use of a definition, or the name that defines it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    pub range: TextRange,
    /// Whether the range is the name that defines the definition, rather than a use of it.
    pub definition: bool,
}

/// The identity of a definition in the script's text: two names refer to the same definition
/// when their keys are equal.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    /// The columns of one table share the range of the table as their definition, so a column
    /// is its definition's range and its name, in lower case, since GoogleSQL matches names
    /// without regard to case.
    Column {
        range: TextRange,
        name: String,
    },
    RangeVariable(TextRange),
    Query(TextRange),
    Variable(TextRange),
    Function(TextRange),
}

impl Key {
    /// The key of `definition`, when the script's text defines it.
    fn of(definition: &Definition) -> Option<Key> {
        Some(match definition {
            Definition::Column(column) => Key::Column {
                range: column.definition?,
                name: column.name.to_ascii_lowercase(),
            },
            Definition::RangeVariable { definition, .. } => Key::RangeVariable((*definition)?),
            Definition::Query { definition, .. } => Key::Query((*definition)?),
            Definition::Variable { definition, .. } => Key::Variable((*definition)?),
            Definition::Function { definition, .. } => Key::Function((*definition)?),
            Definition::Field { .. } | Definition::Table { .. } | Definition::Module { .. } => {
                return None
            }
        })
    }

    fn range(&self) -> TextRange {
        match self {
            Key::Column { range, .. }
            | Key::RangeVariable(range)
            | Key::Query(range)
            | Key::Variable(range)
            | Key::Function(range) => *range,
        }
    }

    /// The key after a rename to `name` whose edits moved the definition's range to `range`.
    fn renamed(&self, range: TextRange, name: &str) -> Key {
        match self {
            Key::Column { .. } => Key::Column {
                range,
                name: name.to_ascii_lowercase(),
            },
            Key::RangeVariable(_) => Key::RangeVariable(range),
            Key::Query(_) => Key::Query(range),
            Key::Variable(_) => Key::Variable(range),
            Key::Function(_) => Key::Function(range),
        }
    }
}

impl Document {
    /// The name at `offset` and the range that defines it. A table of the catalog, a module, a
    /// field, and a function of the library or of a module have no definition in the text.
    pub fn definition(&self, offset: Offset) -> Option<DefinitionLink> {
        let name = self.analysis.model.name_at(self.clamp(offset))?;
        let key = Key::of(&name.definition)?;
        Some(DefinitionLink {
            origin: name.range,
            target: key.range(),
        })
    }

    /// Each use of the definition of the name at `offset`, or of the definition whose name is at
    /// `offset`, and the name that defines it, in the order of the text.
    pub fn references(&self, offset: Offset) -> Vec<Reference> {
        let root = self.root();
        let Some((_, key)) = self.key_at(&root, self.clamp(offset)) else {
            return Vec::new();
        };
        references(&root, &self.analysis.model, &key)
    }

    /// The range of the name at `offset` when a rename can change it: an alias of a column or a
    /// range variable, a variable of a `DECLARE`, a pipe `WITH` name, a temporary table, or a
    /// temporary function. Otherwise the reason that it cannot, which an editor shows.
    pub fn prepare_rename(&self, offset: Offset) -> Result<TextRange, String> {
        let root = self.root();
        let offset = self.clamp(offset);
        if let Some((range, key)) = self.key_at(&root, offset) {
            return match defining_name(&root, key.range()) {
                true => Ok(range),
                false => Err(match key {
                    Key::Column { range, .. }
                        if node_at(&root, range, SyntaxKind::PATH).is_some() =>
                    {
                        "A column of a table has no rename".to_string()
                    }
                    Key::Column { .. } => {
                        "The column takes its name from its expression, without an alias to rename"
                            .to_string()
                    }
                    Key::RangeVariable(_) => {
                        "A range variable without an alias has no rename".to_string()
                    }
                    _ => "The name has no definition to rename".to_string(),
                }),
            };
        }
        let Some(name) = self.analysis.model.name_at(offset) else {
            return Err("No name at the cursor".to_string());
        };
        Err(match &name.definition {
            Definition::Table { .. } => "A table of the catalog has no rename",
            Definition::Module { .. } => "A module has no rename",
            Definition::Field { .. } => "A field of a struct has no rename",
            Definition::Function { .. } => "A function of the library or of a module has no rename",
            _ => "The name has no definition in the script",
        }
        .to_string())
    }

    /// The edits that rename the definition at `offset` and each use of it to `name`, which
    /// they write in backticks when it needs them. A rename that would add an error, or would
    /// change the definition that a name refers to, is an error.
    pub fn rename(&self, offset: Offset, name: &str) -> Result<Vec<TextEdit>, String> {
        self.prepare_rename(offset)?;
        if name.is_empty() || name.chars().any(|c| c == '`' || c.is_control()) {
            return Err(format!("`{name}` is not a name"));
        }
        let root = self.root();
        let (_, key) = (self.key_at(&root, self.clamp(offset))).expect("a name to rename");
        let replacement = quote(name);
        let references = references(&root, &self.analysis.model, &key);
        let edits: Vec<TextEdit> = (references.iter())
            .map(|reference| TextEdit {
                range: reference.range,
                replacement: replacement.clone(),
            })
            .collect();

        // The renamed text must mean what the text means: no new error, and the same names
        // refer to the renamed definition.
        let text = apply(&self.text, &edits);
        let parse = avenger_sql_syntax::parse(&text);
        let analysis = avenger_sql::analyze(
            &parse.syntax(),
            self.catalog.as_ref(),
            &self.library,
            &self.options,
        );
        if parse.diagnostics().len() + analysis.diagnostics.len()
            > self.parse.diagnostics().len() + self.analysis.diagnostics.len()
        {
            let message = (analysis.diagnostics.iter())
                .map(|diagnostic| diagnostic.message.clone())
                .chain(parse.diagnostics().iter().map(|d| d.message.clone()))
                .next()
                .unwrap_or_default();
            return Err(format!("The rename to `{name}` makes an error: {message}"));
        }
        let moved = |range: TextRange| moved(&edits, range);
        let renamed = key.renamed(moved(key.range()), name);
        let uses: Vec<TextRange> = (references.iter())
            .filter(|reference| !reference.definition)
            .map(|reference| moved(reference.range))
            .collect();
        let resolved: Vec<TextRange> = (analysis.model.names.iter())
            .filter(|name| Key::of(&name.definition).as_ref() == Some(&renamed))
            .map(|name| name.range)
            .collect();
        if !uses.iter().all(|range| resolved.contains(range)) || resolved.len() != uses.len() {
            return Err(format!(
                "The rename to `{name}` changes the definition that a name refers to"
            ));
        }
        Ok(edits)
    }

    /// The range of the name at `offset` and the key of its definition: a name that refers to a
    /// definition, or a name that defines one, which the model knows by its uses or by the
    /// columns of an operator.
    fn key_at(&self, root: &SyntaxNode, offset: Offset) -> Option<(TextRange, Key)> {
        let model = &self.analysis.model;
        if let Some(name) = model.name_at(offset) {
            return Some((name.range, Key::of(&name.definition)?));
        }
        let defined = (model.names.iter())
            .filter_map(|name| Key::of(&name.definition))
            .chain((model.operators.iter()).flat_map(|scope| {
                (scope.output.iter())
                    .filter_map(|column| Key::of(&Definition::Column(column.clone())))
            }))
            .find(|key| {
                key.range().contains_inclusive(offset) && defining_name(root, key.range())
            })?;
        Some((defined.range(), defined))
    }
}

/// The uses of `key` in `model`, and its defining name, in the order of the text.
fn references(root: &SyntaxNode, model: &Model, key: &Key) -> Vec<Reference> {
    let mut references: Vec<Reference> = (model.names.iter())
        .filter(|name| Key::of(&name.definition).as_ref() == Some(key))
        .map(|name| Reference {
            range: name.range,
            definition: false,
        })
        .collect();
    if defining_name(root, key.range()) {
        references.retain(|reference| reference.range != key.range());
        references.push(Reference {
            range: key.range(),
            definition: true,
        });
    }
    references.sort_by_key(|reference| reference.range.start());
    references.dedup_by_key(|reference| reference.range);
    references
}

/// Whether `range` is a name that the script defines: a `NAME` node, such as an alias or a
/// variable of a `DECLARE`, or the path of a `CREATE TEMP TABLE` or a `CREATE FUNCTION`.
fn defining_name(root: &SyntaxNode, range: TextRange) -> bool {
    node_at(root, range, SyntaxKind::NAME).is_some()
        || node_at(root, range, SyntaxKind::PATH)
            .and_then(|path| path.parent())
            .is_some_and(|parent| {
                matches!(
                    parent.kind(),
                    SyntaxKind::CREATE_TABLE_STMT | SyntaxKind::CREATE_FUNCTION_STMT
                )
            })
}

/// `text` with `edits`, which do not overlap, in the order of their ranges.
fn apply(text: &str, edits: &[TextEdit]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for edit in edits {
        let range = std::ops::Range::<usize>::from(edit.range);
        out.push_str(&text[at..range.start]);
        out.push_str(&edit.replacement);
        at = range.end;
    }
    out.push_str(&text[at..]);
    out
}

/// Where `range` is after `edits`: an edited range becomes the range of its replacement, and
/// another range moves by the edits before it.
fn moved(edits: &[TextEdit], range: TextRange) -> TextRange {
    let mut shift: i64 = 0;
    for edit in edits {
        if edit.range == range {
            let start = (i64::from(u32::from(range.start())) + shift) as u32;
            return TextRange::at(start.into(), (edit.replacement.len() as u32).into());
        }
        if edit.range.end() <= range.start() {
            shift += edit.replacement.len() as i64 - i64::from(u32::from(edit.range.len()));
        }
    }
    let start = (i64::from(u32::from(range.start())) + shift) as u32;
    TextRange::at(start.into(), range.len())
}
