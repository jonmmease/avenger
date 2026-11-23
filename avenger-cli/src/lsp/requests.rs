//! The requests of the client, each from a method of avenger-sql-language: of the document of a
//! file, or of its project for the requests that cross files.

use std::collections::HashMap;
use std::path::Path;

use avenger_sql_language::{Document, Offset, TextEdit};
use avenger_sql_syntax::{format::format, Role as TextRole};
use lsp_server::{ErrorCode, Request, Response};
use lsp_types::{
    request::{
        CodeActionRequest, Completion, DocumentSymbolRequest, FoldingRangeRequest, Formatting,
        GotoDefinition, HoverRequest, PrepareRenameRequest, References, Rename, Request as _,
        SelectionRangeRequest, SemanticTokensFullRequest, SignatureHelpRequest,
    },
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, CompletionItem,
    CompletionList, CompletionParams, CompletionResponse, CompletionTextEdit,
    DocumentFormattingParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    Documentation, FoldingRange, FoldingRangeKind, FoldingRangeParams, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, InsertReplaceEdit, Location,
    MarkupContent, MarkupKind, ParameterInformation, ParameterLabel, PrepareRenameResponse,
    ReferenceParams, RenameParams, SelectionRange, SelectionRangeParams, SemanticToken,
    SemanticTokens, SemanticTokensParams, SemanticTokensResult, SignatureHelp, SignatureHelpParams,
    SignatureInformation, TextDocumentPositionParams, Url, WorkspaceEdit,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::convert::{self, Encoding, Place};
use super::{lsp_diagnostic, relative, ProjectState, Server};
use avenger_sql_project::qmd::{cell_path, split_cell_path};

/// A file of the analysis that a request reads: a script or a module of a project, or a cell of
/// a document of a project, by its path in the analysis, or a document that is analyzed alone.
enum Target<'a> {
    Project {
        root: &'a Path,
        path: String,
        state: &'a ProjectState,
    },
    Loose(&'a Document),
}

impl Target<'_> {
    fn document(&self) -> &Document {
        match self {
            Target::Project { path, state, .. } => {
                (state.analysis.project().document(path)).expect("a file of the project")
            }
            Target::Loose(document) => document,
        }
    }

    /// The hover at `offset`: of a project's file, with the members of its modules.
    fn hover(&self, offset: Offset) -> Option<avenger_sql_language::Hover> {
        match self {
            Target::Project { path, state, .. } => state.analysis.project().hover(path, offset),
            Target::Loose(document) => document.hover(offset),
        }
    }

    /// The completions at `offset`: of a project's file, with the members of its modules.
    fn completions(&self, offset: Offset) -> avenger_sql_language::Completions {
        match self {
            Target::Project { path, state, .. } => {
                state.analysis.project().completions(path, offset)
            }
            Target::Loose(document) => document.completions(offset),
        }
    }

    /// The signature help at `offset`: of a project's file, with the members of its modules.
    fn signature_help(&self, offset: Offset) -> Option<avenger_sql_language::SignatureHelp> {
        match self {
            Target::Project { path, state, .. } => {
                state.analysis.project().signature_help(path, offset)
            }
            Target::Loose(document) => document.signature_help(offset),
        }
    }
}

/// A file of the analysis in a file of the client, and its place there.
struct Part<'a> {
    target: Target<'a>,
    place: Place<'a>,
}

type Answer = Result<Value, (ErrorCode, String)>;

fn params<P: DeserializeOwned>(value: Value) -> Result<P, (ErrorCode, String)> {
    serde_json::from_value(value).map_err(|e| (ErrorCode::InvalidParams, e.to_string()))
}

fn answer<T: serde::Serialize>(value: T) -> Answer {
    serde_json::to_value(value).map_err(|e| (ErrorCode::InternalError, e.to_string()))
}

fn markdown(value: String) -> MarkupContent {
    MarkupContent {
        kind: MarkupKind::Markdown,
        value,
    }
}

impl Server {
    pub(super) fn request(&mut self, request: Request) {
        let Request { id, method, params } = request;
        let answer = match method.as_str() {
            HoverRequest::METHOD => self.hover(params),
            Completion::METHOD => self.completion(params),
            SignatureHelpRequest::METHOD => self.signature_help(params),
            GotoDefinition::METHOD => self.definition(params),
            References::METHOD => self.references(params),
            PrepareRenameRequest::METHOD => self.prepare_rename(params),
            Rename::METHOD => self.rename(params),
            DocumentSymbolRequest::METHOD => self.symbols(params),
            FoldingRangeRequest::METHOD => self.folding(params),
            SelectionRangeRequest::METHOD => self.selection(params),
            SemanticTokensFullRequest::METHOD => self.tokens(params),
            Formatting::METHOD => self.formatting(params),
            CodeActionRequest::METHOD => self.code_actions(params),
            _ => Err((
                ErrorCode::MethodNotFound,
                format!("avenger lsp does not answer {method}"),
            )),
        };
        let response = match answer {
            Ok(result) => Response::new_ok(id, result),
            Err((code, message)) => Response::new_err(id, code as i32, message),
        };
        let _ = self.sender.send(response.into());
    }

    /// The files of the analysis in the file of `uri`, when the server analyzes it: the file
    /// itself for a script or a module, and each cell, in order, for a document.
    fn parts(&self, uri: &Url) -> Vec<Part<'_>> {
        let Some(path) = super::file_path(uri) else {
            return Vec::new();
        };
        let encoding = self.encoding;
        if let Some(document) = self.loose.get(&path) {
            let place = Place::new(document.text(), None, encoding);
            let target = Target::Loose(document);
            return vec![Part { target, place }];
        }
        if let Some(document) = self.loose_documents.get(&path) {
            return (document.qmd.cells.iter().zip(&document.cells))
                .map(|(cell, analysis)| Part {
                    target: Target::Loose(analysis),
                    place: Place::new(&document.qmd.text, Some(cell), encoding),
                })
                .collect();
        }
        let Some((root, state)) = (self.projects.iter()).find(|(root, _)| path.starts_with(root))
        else {
            return Vec::new();
        };
        let Some(path) = relative(root, &path) else {
            return Vec::new();
        };
        if let Some(document) = state.analysis.qmd(&path) {
            return (document.cells.iter().enumerate())
                .map(|(index, cell)| Part {
                    target: Target::Project {
                        root,
                        path: cell_path(&path, index + 1),
                        state,
                    },
                    place: Place::new(&document.text, Some(cell), encoding),
                })
                .collect();
        }
        let Some(document) = state.analysis.project().document(&path) else {
            return Vec::new();
        };
        let place = Place::new(document.text(), None, encoding);
        let target = Target::Project { root, path, state };
        vec![Part { target, place }]
    }

    /// The file of the analysis at a position, its place, and the offset of the position in it,
    /// or `None` outside every file of the analysis, such as between the cells of a document.
    fn at(&self, position: &TextDocumentPositionParams) -> Option<(Part<'_>, Offset)> {
        (self.parts(&position.text_document.uri).into_iter())
            .find_map(|part| Some((part.place.offset(position.position)?, part)))
            .map(|(offset, part)| (part, offset))
    }

    /// The location of the range `range` of the file at `path` of the analysis of the project
    /// of `state`, whose root is `root`: in its document for a cell.
    fn location(
        &self,
        root: &Path,
        state: &ProjectState,
        path: &str,
        range: avenger_sql_language::TextRange,
    ) -> Option<Location> {
        let (file, place) = self.place_of(root, state, path)?;
        Some(Location::new(file, place.range(range)))
    }

    /// The file of the client of the file at `path` of the analysis of the project of `state`,
    /// and its place there.
    fn place_of<'a>(
        &self,
        root: &Path,
        state: &'a ProjectState,
        path: &str,
    ) -> Option<(Url, Place<'a>)> {
        match split_cell_path(path) {
            Some((document, index)) => {
                let qmd = state.analysis.qmd(document)?;
                let cell = qmd.cells.get(index)?;
                let uri = Url::from_file_path(root.join(document)).ok()?;
                Some((uri, Place::new(&qmd.text, Some(cell), self.encoding)))
            }
            None => {
                let document = state.analysis.project().document(path)?;
                let uri = Url::from_file_path(root.join(path)).ok()?;
                Some((uri, Place::new(document.text(), None, self.encoding)))
            }
        }
    }

    fn hover(&self, value: Value) -> Answer {
        let params: HoverParams = params(value)?;
        let Some((part, offset)) = self.at(&params.text_document_position_params) else {
            return answer(Value::Null);
        };
        let hover = part.target.hover(offset).map(|hover| Hover {
            contents: HoverContents::Markup(markdown(hover.markdown)),
            range: Some(part.place.range(hover.range)),
        });
        answer(hover)
    }

    fn completion(&self, value: Value) -> Answer {
        let params: CompletionParams = params(value)?;
        let Some((part, offset)) = self.at(&params.text_document_position) else {
            return answer(Value::Null);
        };
        let completions = part.target.completions(offset);
        let place = &part.place;
        let (insert, replace) = (
            place.range(completions.range),
            place.range(completions.replace),
        );
        let items = (completions.items.into_iter())
            .map(|item| {
                let text = place.replacement(&item.insert);
                let edit = match self.insert_replace {
                    true => CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                        new_text: text,
                        insert,
                        replace,
                    }),
                    false => CompletionTextEdit::Edit(lsp_types::TextEdit::new(replace, text)),
                };
                CompletionItem {
                    label: item.label,
                    kind: Some(convert::completion_kind(item.kind)),
                    detail: item.detail,
                    documentation: item
                        .documentation
                        .map(|text| Documentation::MarkupContent(markdown(text))),
                    sort_text: Some(item.sort),
                    text_edit: Some(edit),
                    ..Default::default()
                }
            })
            .collect();
        answer(CompletionResponse::List(CompletionList {
            is_incomplete: false,
            items,
        }))
    }

    fn signature_help(&self, value: Value) -> Answer {
        let params: SignatureHelpParams = params(value)?;
        let Some((part, offset)) = self.at(&params.text_document_position_params) else {
            return answer(Value::Null);
        };
        let Some(help) = part.target.signature_help(offset) else {
            return answer(Value::Null);
        };
        let units = |text: &str| match self.encoding {
            Encoding::Utf8 => text.len() as u32,
            Encoding::Utf16 => text.encode_utf16().count() as u32,
            Encoding::Utf32 => text.chars().count() as u32,
        };
        let signatures = (help.signatures.iter())
            .map(|signature| SignatureInformation {
                label: signature.label.clone(),
                documentation: (signature.documentation.clone())
                    .map(|text| Documentation::MarkupContent(markdown(text))),
                parameters: Some(
                    (signature.parameters.iter())
                        .map(|parameter| ParameterInformation {
                            label: ParameterLabel::LabelOffsets([
                                units(&signature.label[..parameter.range.start]),
                                units(&signature.label[..parameter.range.end]),
                            ]),
                            documentation: (parameter.documentation.clone())
                                .map(|text| Documentation::MarkupContent(markdown(text))),
                        })
                        .collect(),
                ),
                active_parameter: signature.active_parameter.map(|at| at as u32),
            })
            .collect();
        answer(SignatureHelp {
            signatures,
            active_signature: Some(help.active_signature as u32),
            active_parameter: None,
        })
    }

    fn definition(&self, value: Value) -> Answer {
        let params: GotoDefinitionParams = params(value)?;
        let position = &params.text_document_position_params;
        let Some((part, offset)) = self.at(position) else {
            return answer(Value::Null);
        };
        let location = match &part.target {
            Target::Project { root, path, state } => {
                (state.analysis.project().definition(path, offset))
                    .and_then(|link| self.location(root, state, &link.path, link.target))
            }
            Target::Loose(document) => (document.definition(offset)).map(|link| {
                let uri = position.text_document.uri.clone();
                Location::new(uri, part.place.range(link.target))
            }),
        };
        answer(location.map(GotoDefinitionResponse::Scalar))
    }

    fn references(&self, value: Value) -> Answer {
        let params: ReferenceParams = params(value)?;
        let position = &params.text_document_position;
        let Some((part, offset)) = self.at(position) else {
            return answer(Value::Null);
        };
        let declarations = params.context.include_declaration;
        let locations: Vec<Location> = match &part.target {
            Target::Project { root, path, state } => (state
                .analysis
                .project()
                .references(path, offset)
                .into_iter())
            .filter(|reference| declarations || !reference.definition)
            .filter_map(|reference| self.location(root, state, &reference.path, reference.range))
            .collect(),
            Target::Loose(document) => (document.references(offset).into_iter())
                .filter(|reference| declarations || !reference.definition)
                .map(|reference| {
                    let uri = position.text_document.uri.clone();
                    Location::new(uri, part.place.range(reference.range))
                })
                .collect(),
        };
        answer(locations)
    }

    fn prepare_rename(&self, value: Value) -> Answer {
        let params: TextDocumentPositionParams = params(value)?;
        let Some((part, offset)) = self.at(&params) else {
            return answer(Value::Null);
        };
        let range = match &part.target {
            Target::Project { path, state, .. } => {
                state.analysis.project().prepare_rename(path, offset)
            }
            Target::Loose(document) => document.prepare_rename(offset),
        };
        match range {
            Ok(range) => answer(PrepareRenameResponse::Range(part.place.range(range))),
            Err(message) => Err((ErrorCode::RequestFailed, message)),
        }
    }

    fn rename(&self, value: Value) -> Answer {
        let params: RenameParams = params(value)?;
        let position = &params.text_document_position;
        let Some((part, offset)) = self.at(position) else {
            return answer(Value::Null);
        };
        let name = &params.new_name;
        let mut changes: HashMap<Url, Vec<lsp_types::TextEdit>> = HashMap::new();
        match &part.target {
            Target::Project { root, path, state } => {
                let edits = (state.analysis.project().rename(path, offset, name))
                    .map_err(|message| (ErrorCode::RequestFailed, message))?;
                for (path, edits) in edits {
                    let Some((uri, place)) = self.place_of(root, state, &path) else {
                        continue;
                    };
                    // The cells of a document add their edits to the document's.
                    (changes.entry(uri).or_default()).extend(edits_at(&place, &edits));
                }
            }
            Target::Loose(document) => {
                let edits = (document.rename(offset, name))
                    .map_err(|message| (ErrorCode::RequestFailed, message))?;
                let uri = position.text_document.uri.clone();
                changes.insert(uri, edits_at(&part.place, &edits));
            }
        }
        answer(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        })
    }

    fn symbols(&self, value: Value) -> Answer {
        let params: DocumentSymbolParams = params(value)?;
        let parts = self.parts(&params.text_document.uri);
        if parts.is_empty() {
            return answer(Value::Null);
        }
        #[allow(deprecated)]
        fn symbol(place: &Place, outline: &avenger_sql_language::Symbol) -> DocumentSymbol {
            DocumentSymbol {
                name: outline.name.clone(),
                detail: outline.detail.clone(),
                kind: convert::symbol_kind(outline.kind),
                tags: None,
                deprecated: None,
                range: place.range(outline.range),
                selection_range: place.range(outline.name_range),
                children: Some(outline.children.iter().map(|c| symbol(place, c)).collect()),
            }
        }
        let symbols = (parts.iter())
            .flat_map(|part| {
                let symbols = part.target.document().symbols();
                symbols
                    .iter()
                    .map(|s| symbol(&part.place, s))
                    .collect::<Vec<_>>()
            })
            .collect();
        answer(DocumentSymbolResponse::Nested(symbols))
    }

    fn folding(&self, value: Value) -> Answer {
        let params: FoldingRangeParams = params(value)?;
        let parts = self.parts(&params.text_document.uri);
        if parts.is_empty() {
            return answer(Value::Null);
        }
        let mut ranges: Vec<FoldingRange> = Vec::new();
        for part in &parts {
            for folding in part.target.document().folding_ranges().iter() {
                let range = part.place.range(folding.range);
                if range.end.line <= range.start.line {
                    continue;
                }
                ranges.push(FoldingRange {
                    start_line: range.start.line,
                    start_character: None,
                    end_line: range.end.line,
                    end_character: None,
                    kind: Some(match folding.comment {
                        true => FoldingRangeKind::Comment,
                        false => FoldingRangeKind::Region,
                    }),
                    collapsed_text: None,
                });
            }
        }
        answer(ranges)
    }

    fn selection(&self, value: Value) -> Answer {
        let params: SelectionRangeParams = params(value)?;
        let parts = self.parts(&params.text_document.uri);
        if parts.is_empty() {
            return answer(Value::Null);
        }
        let ranges: Vec<SelectionRange> = (params.positions.iter())
            .map(|position| {
                let found =
                    (parts.iter()).find_map(|part| Some((part, part.place.offset(*position)?)));
                // Innermost first, so the outermost is the last parent.
                let mut selection: Option<SelectionRange> = None;
                if let Some((part, offset)) = found {
                    for range in part.target.document().selection_ranges(offset).iter().rev() {
                        selection = Some(SelectionRange {
                            range: part.place.range(*range),
                            parent: selection.map(Box::new),
                        });
                    }
                }
                selection.unwrap_or(SelectionRange {
                    range: lsp_types::Range::new(*position, *position),
                    parent: None,
                })
            })
            .collect();
        answer(ranges)
    }

    fn tokens(&self, value: Value) -> Answer {
        let params: SemanticTokensParams = params(value)?;
        let parts = self.parts(&params.text_document.uri);
        if parts.is_empty() {
            return answer(Value::Null);
        }
        let mut data = Vec::new();
        let (mut line, mut start) = (0, 0);
        for part in &parts {
            let document = part.target.document();
            let text = document.text();
            // The strings in a language of their own take no tokens: an editor's Tree-sitter
            // grammar highlights them by injection, and a token would cover its colors.
            let embedded = document.embedded_strings();
            for highlight in document.highlights() {
                let inside = |string: &avenger_sql_language::EmbeddedString| {
                    string.range.contains_range(highlight.range)
                };
                if embedded.iter().any(inside) {
                    continue;
                }
                let Some(kind) = convert::token_type(highlight.highlight) else {
                    continue;
                };
                let modifiers = u32::from(highlight.definition);
                // A token of one line each, since a client need not take tokens over lines.
                let (from, to) = (
                    usize::from(highlight.range.start()),
                    usize::from(highlight.range.end()),
                );
                let mut piece = from;
                for content in text[from..to].split_inclusive('\n') {
                    let at = piece;
                    piece += content.len();
                    let content = content.trim_end_matches(['\n', '\r']);
                    if content.is_empty() {
                        continue;
                    }
                    let position = part.place.position(at);
                    let length = match self.encoding {
                        Encoding::Utf8 => content.len(),
                        Encoding::Utf16 => content.encode_utf16().count(),
                        Encoding::Utf32 => content.chars().count(),
                    } as u32;
                    let delta_line = position.line - line;
                    let delta_start = match delta_line {
                        0 => position.character - start,
                        _ => position.character,
                    };
                    data.push(SemanticToken {
                        delta_line,
                        delta_start,
                        length,
                        token_type: kind,
                        token_modifiers_bitset: modifiers,
                    });
                    (line, start) = (position.line, position.character);
                }
            }
        }
        answer(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        }))
    }

    /// The edits that format the file: of a script or a module, by its document, and of a
    /// document, of each cell that changes, with the cell's indent, as `avenger fmt` formats it.
    fn formatting(&self, value: Value) -> Answer {
        let params: DocumentFormattingParams = params(value)?;
        let parts = self.parts(&params.text_document.uri);
        if parts.is_empty() {
            return answer(Value::Null);
        }
        let mut edits = Vec::new();
        for part in &parts {
            let document = part.target.document();
            match part.place.cell() {
                None => edits.extend(edits_at(&part.place, &document.formatting())),
                Some(cell) => {
                    let formatted = format(&cell.text, TextRole::Script);
                    if formatted == cell.text {
                        continue;
                    }
                    let range = lsp_types::Range::new(
                        part.place.document_position(cell.range.start),
                        part.place.document_position(cell.range.end),
                    );
                    edits.push(lsp_types::TextEdit::new(range, cell.indented(&formatted)));
                }
            }
        }
        answer(edits)
    }

    /// The quick fixes of the diagnostics of the file whose ranges meet the request's range.
    fn code_actions(&self, value: Value) -> Answer {
        let params: CodeActionParams = params(value)?;
        let uri = &params.text_document.uri;
        let mut actions = Vec::new();
        for part in self.parts(uri) {
            let Some(range) = part.place.text_range(params.range) else {
                continue;
            };
            let diagnostics = match &part.target {
                Target::Project { path, state, .. } => state.analysis.project().diagnostics(path),
                Target::Loose(document) => document.diagnostics(),
            };
            for diagnostic in &diagnostics {
                let meets = diagnostic.range.start() <= range.end()
                    && range.start() <= diagnostic.range.end();
                if !meets {
                    continue;
                }
                for fix in &diagnostic.fixes {
                    let edit = WorkspaceEdit {
                        changes: Some(HashMap::from([(
                            uri.clone(),
                            edits_at(&part.place, &fix.edits),
                        )])),
                        ..Default::default()
                    };
                    actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                        title: fix.label.clone(),
                        kind: Some(CodeActionKind::QUICKFIX),
                        diagnostics: Some(vec![lsp_diagnostic(&part.place, diagnostic)]),
                        edit: Some(edit),
                        ..Default::default()
                    }));
                }
            }
        }
        answer(actions)
    }
}

/// The edits of the protocol of `edits` of a file of the analysis at `place`.
fn edits_at(place: &Place, edits: &[TextEdit]) -> Vec<lsp_types::TextEdit> {
    (edits.iter())
        .map(|edit| {
            lsp_types::TextEdit::new(
                place.range(edit.range),
                place.replacement(&edit.replacement),
            )
        })
        .collect()
}
