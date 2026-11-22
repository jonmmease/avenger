//! The requests of the client, each from a method of avenger-sql-language: of the document of a
//! file, or of its project for the requests that cross files.

use std::collections::HashMap;
use std::path::Path;

use avenger_sql_language::{Document, Offset, Project, TextEdit};
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

use super::convert::{self, Encoding, LineIndex};
use super::{lsp_diagnostic, relative, Server};

/// The file of a request: a script or a module of a project, or a document that is analyzed
/// alone.
enum Target<'a> {
    Project {
        root: &'a Path,
        path: String,
        project: &'a Project,
    },
    Loose(&'a Document),
}

impl Target<'_> {
    fn document(&self) -> Option<&Document> {
        match self {
            Target::Project { path, project, .. } => project.document(path),
            Target::Loose(document) => Some(document),
        }
    }
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

    /// The file of `uri`, when the server analyzes it.
    fn target(&self, uri: &Url) -> Option<Target<'_>> {
        let path = super::file_path(uri)?;
        if let Some(document) = self.loose.get(&path) {
            return Some(Target::Loose(document));
        }
        let (root, state) = self
            .projects
            .iter()
            .find(|(root, _)| path.starts_with(root))?;
        let path = relative(root, &path)?;
        let project = state.analysis.project();
        project.document(&path)?;
        Some(Target::Project {
            root,
            path,
            project,
        })
    }

    /// The file and the byte offset of a request at a position.
    fn at(&self, position: &TextDocumentPositionParams) -> Option<(Target<'_>, Offset)> {
        let target = self.target(&position.text_document.uri)?;
        let index = LineIndex::new(target.document()?.text(), self.encoding);
        let offset = index.offset(position.position);
        Some((target, offset))
    }

    fn index<'a>(&self, document: &'a Document) -> LineIndex<'a> {
        LineIndex::new(document.text(), self.encoding)
    }

    fn hover(&self, value: Value) -> Answer {
        let params: HoverParams = params(value)?;
        let Some((target, offset)) = self.at(&params.text_document_position_params) else {
            return answer(Value::Null);
        };
        let document = target.document().expect("a document");
        let hover = document.hover(offset).map(|hover| Hover {
            contents: HoverContents::Markup(markdown(hover.markdown)),
            range: Some(self.index(document).range(hover.range)),
        });
        answer(hover)
    }

    fn completion(&self, value: Value) -> Answer {
        let params: CompletionParams = params(value)?;
        let Some((target, offset)) = self.at(&params.text_document_position) else {
            return answer(Value::Null);
        };
        let document = target.document().expect("a document");
        let completions = document.completions(offset);
        let index = self.index(document);
        let (insert, replace) = (
            index.range(completions.range),
            index.range(completions.replace),
        );
        let items = (completions.items.into_iter())
            .map(|item| {
                let edit = match self.insert_replace {
                    true => CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                        new_text: item.insert.clone(),
                        insert,
                        replace,
                    }),
                    false => CompletionTextEdit::Edit(lsp_types::TextEdit::new(
                        replace,
                        item.insert.clone(),
                    )),
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
        let Some((target, offset)) = self.at(&params.text_document_position_params) else {
            return answer(Value::Null);
        };
        let document = target.document().expect("a document");
        let Some(help) = document.signature_help(offset) else {
            return answer(Value::Null);
        };
        let units = |text: &str| match self.encoding {
            Encoding::Utf8 => text.len() as u32,
            Encoding::Utf16 => text.encode_utf16().count() as u32,
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

    /// The location of the range `range` of the file at `path` of the project at `root`.
    fn location(
        &self,
        root: &Path,
        project: &Project,
        path: &str,
        range: avenger_sql_language::TextRange,
    ) -> Option<Location> {
        let document = project.document(path)?;
        let uri = Url::from_file_path(root.join(path)).ok()?;
        Some(Location::new(uri, self.index(document).range(range)))
    }

    fn definition(&self, value: Value) -> Answer {
        let params: GotoDefinitionParams = params(value)?;
        let position = &params.text_document_position_params;
        let Some((target, offset)) = self.at(position) else {
            return answer(Value::Null);
        };
        let location = match &target {
            Target::Project {
                root,
                path,
                project,
            } => (project.definition(path, offset))
                .and_then(|link| self.location(root, project, &link.path, link.target)),
            Target::Loose(document) => (document.definition(offset)).map(|link| {
                let uri = position.text_document.uri.clone();
                Location::new(uri, self.index(document).range(link.target))
            }),
        };
        answer(location.map(GotoDefinitionResponse::Scalar))
    }

    fn references(&self, value: Value) -> Answer {
        let params: ReferenceParams = params(value)?;
        let position = &params.text_document_position;
        let Some((target, offset)) = self.at(position) else {
            return answer(Value::Null);
        };
        let declarations = params.context.include_declaration;
        let locations: Vec<Location> = match &target {
            Target::Project {
                root,
                path,
                project,
            } => (project.references(path, offset).into_iter())
                .filter(|reference| declarations || !reference.definition)
                .filter_map(|reference| {
                    self.location(root, project, &reference.path, reference.range)
                })
                .collect(),
            Target::Loose(document) => (document.references(offset).into_iter())
                .filter(|reference| declarations || !reference.definition)
                .map(|reference| {
                    let uri = position.text_document.uri.clone();
                    Location::new(uri, self.index(document).range(reference.range))
                })
                .collect(),
        };
        answer(locations)
    }

    fn prepare_rename(&self, value: Value) -> Answer {
        let params: TextDocumentPositionParams = params(value)?;
        let Some((target, offset)) = self.at(&params) else {
            return answer(Value::Null);
        };
        let range = match &target {
            Target::Project { path, project, .. } => project.prepare_rename(path, offset),
            Target::Loose(document) => document.prepare_rename(offset),
        };
        let document = target.document().expect("a document");
        match range {
            Ok(range) => answer(PrepareRenameResponse::Range(
                self.index(document).range(range),
            )),
            Err(message) => Err((ErrorCode::RequestFailed, message)),
        }
    }

    fn rename(&self, value: Value) -> Answer {
        let params: RenameParams = params(value)?;
        let position = &params.text_document_position;
        let Some((target, offset)) = self.at(position) else {
            return answer(Value::Null);
        };
        let name = &params.new_name;
        let mut changes: HashMap<Url, Vec<lsp_types::TextEdit>> = HashMap::new();
        match &target {
            Target::Project {
                root,
                path,
                project,
            } => {
                let edits = (project.rename(path, offset, name))
                    .map_err(|message| (ErrorCode::RequestFailed, message))?;
                for (path, edits) in edits {
                    let (Some(document), Ok(uri)) = (
                        project.document(&path),
                        Url::from_file_path(root.join(&path)),
                    ) else {
                        continue;
                    };
                    changes.insert(uri, self.edits(document, &edits));
                }
            }
            Target::Loose(document) => {
                let edits = (document.rename(offset, name))
                    .map_err(|message| (ErrorCode::RequestFailed, message))?;
                let uri = position.text_document.uri.clone();
                changes.insert(uri, self.edits(document, &edits));
            }
        }
        answer(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        })
    }

    fn edits(&self, document: &Document, edits: &[TextEdit]) -> Vec<lsp_types::TextEdit> {
        let index = self.index(document);
        (edits.iter())
            .map(|edit| lsp_types::TextEdit::new(index.range(edit.range), edit.replacement.clone()))
            .collect()
    }

    fn document_of(&self, uri: &Url) -> Option<&Document> {
        match self.target(uri)? {
            Target::Project { path, project, .. } => project.document(&path),
            Target::Loose(document) => Some(document),
        }
    }

    fn symbols(&self, value: Value) -> Answer {
        let params: DocumentSymbolParams = params(value)?;
        let Some(document) = self.document_of(&params.text_document.uri) else {
            return answer(Value::Null);
        };
        let index = self.index(document);
        #[allow(deprecated)]
        fn symbol(index: &LineIndex, outline: &avenger_sql_language::Symbol) -> DocumentSymbol {
            DocumentSymbol {
                name: outline.name.clone(),
                detail: outline.detail.clone(),
                kind: convert::symbol_kind(outline.kind),
                tags: None,
                deprecated: None,
                range: index.range(outline.range),
                selection_range: index.range(outline.name_range),
                children: Some(outline.children.iter().map(|c| symbol(index, c)).collect()),
            }
        }
        let symbols = (document.symbols().iter())
            .map(|s| symbol(&index, s))
            .collect();
        answer(DocumentSymbolResponse::Nested(symbols))
    }

    fn folding(&self, value: Value) -> Answer {
        let params: FoldingRangeParams = params(value)?;
        let Some(document) = self.document_of(&params.text_document.uri) else {
            return answer(Value::Null);
        };
        let index = self.index(document);
        let ranges: Vec<FoldingRange> = (document.folding_ranges().iter())
            .map(|folding| {
                let range = index.range(folding.range);
                FoldingRange {
                    start_line: range.start.line,
                    start_character: None,
                    end_line: range.end.line,
                    end_character: None,
                    kind: Some(match folding.comment {
                        true => FoldingRangeKind::Comment,
                        false => FoldingRangeKind::Region,
                    }),
                    collapsed_text: None,
                }
            })
            .filter(|range| range.end_line > range.start_line)
            .collect();
        answer(ranges)
    }

    fn selection(&self, value: Value) -> Answer {
        let params: SelectionRangeParams = params(value)?;
        let Some(document) = self.document_of(&params.text_document.uri) else {
            return answer(Value::Null);
        };
        let index = self.index(document);
        let ranges: Vec<SelectionRange> = (params.positions.iter())
            .map(|position| {
                let offset = index.offset(*position);
                // Innermost first, so the outermost is the last parent.
                let mut selection: Option<SelectionRange> = None;
                for range in document.selection_ranges(offset).iter().rev() {
                    selection = Some(SelectionRange {
                        range: index.range(*range),
                        parent: selection.map(Box::new),
                    });
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
        let Some(document) = self.document_of(&params.text_document.uri) else {
            return answer(Value::Null);
        };
        let text = document.text();
        let index = self.index(document);
        let mut data = Vec::new();
        let (mut line, mut start) = (0, 0);
        for highlight in document.highlights() {
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
            for part in text[from..to].split_inclusive('\n') {
                let content = part.trim_end_matches(['\n', '\r']);
                if !content.is_empty() {
                    let position = index.position(piece);
                    let length = match self.encoding {
                        Encoding::Utf8 => content.len(),
                        Encoding::Utf16 => content.encode_utf16().count(),
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
                piece += part.len();
            }
        }
        answer(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        }))
    }

    fn formatting(&self, value: Value) -> Answer {
        let params: DocumentFormattingParams = params(value)?;
        let Some(document) = self.document_of(&params.text_document.uri) else {
            return answer(Value::Null);
        };
        answer(self.edits(document, &document.formatting()))
    }

    /// The quick fixes of the diagnostics of the file whose ranges meet the request's range.
    fn code_actions(&self, value: Value) -> Answer {
        let params: CodeActionParams = params(value)?;
        let uri = &params.text_document.uri;
        let Some(target) = self.target(uri) else {
            return answer(Value::Null);
        };
        let document = target.document().expect("a document");
        let diagnostics = match &target {
            Target::Project { path, project, .. } => project.diagnostics(path),
            Target::Loose(document) => document.diagnostics(),
        };
        let index = self.index(document);
        let range = index.text_range(params.range);
        let mut actions = Vec::new();
        for diagnostic in &diagnostics {
            let meets =
                diagnostic.range.start() <= range.end() && range.start() <= diagnostic.range.end();
            if !meets {
                continue;
            }
            for fix in &diagnostic.fixes {
                let edit = WorkspaceEdit {
                    changes: Some(HashMap::from([(
                        uri.clone(),
                        self.edits(document, &fix.edits),
                    )])),
                    ..Default::default()
                };
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: fix.label.clone(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![lsp_diagnostic(&index, diagnostic)]),
                    edit: Some(edit),
                    ..Default::default()
                }));
            }
        }
        answer(actions)
    }
}
