//! A position of a file, `PATH:LINE:COLUMN`, as the commands of positions take it: `docs --at`,
//! `complete`, and `signature`. Lines and columns start at 1, and a column counts characters, as
//! the diagnostics print them. A script, a data script, or a module of a project is a file of its
//! project's analysis, a position of a Quarto document is in the `{avenger-sql}` cell that holds
//! it, and a loose file analyzes alone, as `check` analyzes it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
use avenger_sql_language::{Document, Offset, Role as TextRole};
use avenger_sql_project::{
    find_root,
    qmd::{cell_path, is_document, Cell, Qmd},
    MODULE_SUFFIX,
};
use lsp_types::Position;

use crate::{
    lsp::convert::{Encoding, LineIndex, Place},
    project::{open, Opened},
};

/// The analysis that holds the document of a position.
enum Analyzed {
    /// A file of a project, by its path in the project's analysis.
    Project { opened: Box<Opened>, path: String },
    /// A loose file, or a file without a role in its project, alone.
    Alone(Box<Document>),
}

/// The document and the offset of a position.
pub(crate) struct At {
    analyzed: Analyzed,
    /// The text of the file on disk, and the cell of a document that holds the position.
    text: String,
    cell: Option<Cell>,
    /// The offset in the document.
    pub offset: Offset,
}

impl At {
    /// The position `at`, `PATH:LINE:COLUMN`, of a file.
    pub async fn resolve(at: &str) -> Result<At, String> {
        let usage = || format!("`{at}`: a position is `PATH:LINE:COLUMN`, from 1");
        let mut parts = at.rsplitn(3, ':');
        let (column, line, path) = (parts.next(), parts.next(), parts.next());
        let (Some(column), Some(line), Some(path)) = (column, line, path) else {
            return Err(usage());
        };
        let number = |text: &str| text.parse::<u32>().ok().filter(|n| *n >= 1);
        let (Some(line), Some(column)) = (number(line), number(column)) else {
            return Err(usage());
        };
        let position = Position::new(line - 1, column - 1);
        let file = std::fs::canonicalize(path).map_err(|error| format!("{path}: {error}"))?;
        let text = std::fs::read_to_string(&file).map_err(|error| format!("{path}: {error}"))?;
        if let Some(root) = find_root(&file) {
            let opened = open(Some(&file)).await?;
            let relative = relative(&root, &file);
            if let Some(qmd) = opened.analysis.qmd(&relative) {
                let (index, cell, offset) = in_cell(qmd, position, path)?;
                let path = cell_path(&relative, index + 1);
                let opened = Box::new(opened);
                return Ok(At {
                    analyzed: Analyzed::Project { opened, path },
                    text,
                    cell: Some(cell),
                    offset,
                });
            }
            if opened.analysis.project().document(&relative).is_some() {
                let offset = LineIndex::new(&text, Encoding::Utf32).offset(position);
                let (opened, path) = (Box::new(opened), relative);
                return Ok(At {
                    analyzed: Analyzed::Project { opened, path },
                    text,
                    cell: None,
                    offset,
                });
            }
        }
        // A loose file, or a file without a role in its project, alone.
        if is_document(&file.to_string_lossy()) {
            let (_, cell, offset) = in_cell(&Qmd::parse(text.as_str()), position, path)?;
            return Ok(At {
                analyzed: Analyzed::Alone(Box::new(alone(&cell.text, TextRole::Script))),
                text,
                cell: Some(cell),
                offset,
            });
        }
        let role = match file.to_string_lossy().ends_with(MODULE_SUFFIX) {
            true => TextRole::Module,
            false => TextRole::Script,
        };
        let offset = LineIndex::new(&text, Encoding::Utf32).offset(position);
        Ok(At {
            analyzed: Analyzed::Alone(Box::new(alone(&text, role))),
            text,
            cell: None,
            offset,
        })
    }

    /// The document of the position.
    pub fn document(&self) -> &Document {
        match &self.analyzed {
            Analyzed::Project { opened, path } => {
                (opened.analysis.project().document(path)).expect("the document of the position")
            }
            Analyzed::Alone(document) => document,
        }
    }

    /// The line and the column, from 1, of the offset `offset` of the document.
    pub fn line_column(&self, offset: Offset) -> (u32, u32) {
        let place = Place::new(&self.text, self.cell.as_ref(), Encoding::Utf32);
        let position = place.position(usize::from(offset));
        (position.line + 1, position.character + 1)
    }
}

/// The cell of `qmd` that holds `position`, its index, and the position's offset in it, or the
/// error of a position of the file `path` outside every cell.
fn in_cell(qmd: &Qmd, position: Position, path: &str) -> Result<(usize, Cell, Offset), String> {
    let offset = usize::from(LineIndex::new(&qmd.text, Encoding::Utf32).offset(position));
    let found = (qmd.cells.iter().enumerate())
        .find_map(|(index, cell)| Some((index, cell, cell.to_cell(offset)?)));
    match found {
        Some((index, cell, in_cell)) => Ok((index, cell.clone(), Offset::from(in_cell as u32))),
        None => Err(format!(
            "{path}:{}:{}: not in an `{{avenger-sql}}` cell",
            position.line + 1,
            position.character + 1
        )),
    }
}

/// The path of `file` relative to `root`, with `/`.
fn relative(root: &Path, file: &Path) -> String {
    let relative: PathBuf = file.strip_prefix(root).unwrap_or(file).to_path_buf();
    relative.to_string_lossy().replace('\\', "/")
}

/// The analysis of `text` alone, with no tables but its own, as `check` analyzes a loose file.
fn alone(text: &str, role: TextRole) -> Document {
    Document::new(
        text,
        role,
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    )
}
