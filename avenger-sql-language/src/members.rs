//! The members of a project's modules: a module's `CREATE PUBLIC` functions and constants, which a
//! script, a cell, or another module reaches by a qualified name after it imports the module, as
//! `weather.warm` after `IMPORT MODULE weather`. The qualifier is the import's alias, or the last
//! name of the module's path without one, and the member is the public statement of its name in
//! the module's file. Definition, hover, signature help, completion, references, and rename read
//! the one resolution here, so they agree.

use std::collections::BTreeMap;

use avenger_sql::analysis::Definition;
use avenger_sql_syntax::{
    ast::{self, AstNode, CreateStmt, Scope},
    FunctionRole, SyntaxKind, SyntaxNode, TextEdit, TextRange,
};

use crate::{
    completion::{CompletionItem, CompletionKind, Completions},
    hover::{create_header, Hover},
    navigation::check_name,
    project::{FileRole, ProjectLink, ProjectReference},
    quote,
    signature::{call_at, function_help, SignatureHelp},
    token_before, trimmed, Document, Offset, Project,
};

/// The statement of a member.
#[derive(Clone, Debug)]
pub(crate) enum Statement {
    Function(ast::CreateFunctionStmt),
    Constant(ast::CreateConstantStmt),
}

/// A statement of a module that a qualified name can reach, or that only its module reads: its
/// name, the range of its name in the module's file, its statement, and whether it is public.
#[derive(Clone, Debug)]
pub(crate) struct Member {
    pub(crate) name: String,
    pub(crate) range: TextRange,
    pub(crate) statement: Statement,
    pub(crate) public: bool,
}

/// A qualified name of a document whose qualifier an import of the document names: the ranges of
/// the qualifier and of the member's name, the member's name as written, and the import.
#[derive(Clone, Debug)]
pub(crate) struct Qualified {
    pub(crate) qualifier: TextRange,
    pub(crate) member: TextRange,
    pub(crate) name: String,
    pub(crate) import: ast::ImportStmt,
}

/// Which part of a qualified name holds an offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Qualifier,
    Member,
}

/// A member at an offset of a document: the module's path, the member, and the qualified name
/// and its part that hold the offset, or none at the member's own name in its module.
pub(crate) struct Found {
    pub(crate) module: String,
    pub(crate) member: Member,
    pub(crate) at: Option<(Qualified, Part)>,
}

/// The name that an import's qualified names start with: its alias, or the last name of its
/// module's path.
fn qualifier(import: &ast::ImportStmt) -> Option<String> {
    match import.alias().and_then(|alias| alias.name()) {
        Some(name) => name.text(),
        None => import.path()?.names()?.pop(),
    }
}

/// The `IMPORT MODULE` statements of `root`.
fn imports(root: &SyntaxNode) -> Vec<ast::ImportStmt> {
    root.descendants()
        .filter_map(ast::ImportStmt::cast)
        .collect()
}

/// The members of the module whose root is `root`, public and private, in the order of its text.
fn module_members(root: &SyntaxNode) -> Vec<Member> {
    let Some(module) = ast::Module::cast(root.clone()) else {
        return Vec::new();
    };
    let member = |path: Option<ast::Path>, statement: Statement, scope: Option<Scope>| {
        let path = path?;
        let mut names = path.names()?;
        (names.len() == 1).then(|| Member {
            name: names.pop().expect("a name"),
            range: trimmed(path.syntax()),
            statement,
            public: scope == Some(Scope::Public),
        })
    };
    (module.stmts())
        .filter_map(|stmt| match stmt {
            ast::Stmt::CreateFunctionStmt(create) => member(
                create.name_path(),
                Statement::Function(create.clone()),
                create.scope_kind(),
            ),
            ast::Stmt::CreateConstantStmt(create) => member(
                create.name_path(),
                Statement::Constant(create.clone()),
                create.scope_kind(),
            ),
            _ => None,
        })
        .collect()
}

/// The comment lines just before `node`, with no blank line between them and it, without their
/// `--`, `#`, or `/* */`.
fn leading_comment(node: &SyntaxNode) -> Option<String> {
    let first = (node.descendants_with_tokens())
        .filter_map(|element| element.into_token())
        .find(|token| !token.kind().is_trivia())?;
    let mut lines = Vec::new();
    let mut token = first.prev_token();
    while let Some(current) = token {
        match current.kind() {
            SyntaxKind::WHITESPACE if current.text().matches('\n').count() > 1 => break,
            SyntaxKind::WHITESPACE => {}
            SyntaxKind::COMMENT => {
                let text = current.text();
                let text = (text.strip_prefix("--").or_else(|| text.strip_prefix('#')))
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        let inner = text.trim_start_matches("/*").trim_end_matches("*/");
                        inner.to_string()
                    });
                lines.push(text.trim().to_string());
            }
            _ => break,
        }
        token = current.prev_token();
    }
    lines.reverse();
    let comment = lines.join("\n");
    (!comment.trim().is_empty()).then_some(comment)
}

/// The names of `member` in its module `document` without a qualifier: a call of a function, or
/// a constant in an expression, whose name the analysis doesn't read as a column, a range
/// variable, or a variable.
fn local_uses(document: &Document, member: &Member) -> Vec<TextRange> {
    let model = document.model();
    let parents: &[SyntaxKind] = match member.statement {
        Statement::Function(_) => &[
            SyntaxKind::CALL_EXPR,
            SyntaxKind::TVF_SOURCE,
            SyntaxKind::PIPE_CALL,
        ],
        Statement::Constant(_) => &[SyntaxKind::PATH_EXPR],
    };
    (document.root().descendants())
        .filter_map(ast::Path::cast)
        .filter(|path| {
            (path.syntax().parent()).is_some_and(|parent| parents.contains(&parent.kind()))
        })
        .filter_map(|path| {
            let refs: Vec<ast::NameRef> = path.name_refs().collect();
            let [name] = refs.as_slice() else {
                return None;
            };
            let range = name.syntax().text_range();
            let local = (model.name_at(range.start())).is_some_and(|name| {
                matches!(
                    name.definition,
                    Definition::RangeVariable { .. }
                        | Definition::Column(_)
                        | Definition::Variable { .. }
                )
            });
            (!local && name.text()?.eq_ignore_ascii_case(&member.name)).then_some(range)
        })
        .collect()
}

impl Project {
    /// The path and the document of the module that `import` imports.
    fn imported(&self, import: &ast::ImportStmt) -> Option<(&str, &Document)> {
        let names = import.path()?.names()?;
        self.module_file(&names)
    }

    /// The qualified names of `document` whose qualifiers its imports name, in the order of its
    /// text: a call of a function, a table function, or a pipe `CALL`, and a constant in an
    /// expression. A path whose first name the analysis reads as a range variable or a column is
    /// not one.
    pub(crate) fn qualified_names(&self, document: &Document) -> Vec<Qualified> {
        let root = document.root();
        let imports: Vec<(String, ast::ImportStmt)> = (imports(&root).into_iter())
            .filter_map(|import| Some((qualifier(&import)?.to_lowercase(), import)))
            .collect();
        if imports.is_empty() {
            return Vec::new();
        }
        let model = document.model();
        (root.descendants())
            .filter_map(ast::Path::cast)
            .filter(|path| {
                (path.syntax().parent()).is_some_and(|parent| {
                    matches!(
                        parent.kind(),
                        SyntaxKind::CALL_EXPR
                            | SyntaxKind::PATH_EXPR
                            | SyntaxKind::TVF_SOURCE
                            | SyntaxKind::PIPE_CALL
                    )
                })
            })
            .filter_map(|path| {
                let refs: Vec<ast::NameRef> = path.name_refs().collect();
                let [first, second] = refs.as_slice() else {
                    return None;
                };
                let qualifier = first.syntax().text_range();
                let local = (model.name_at(qualifier.start())).is_some_and(|name| {
                    matches!(
                        name.definition,
                        Definition::RangeVariable { .. } | Definition::Column(_)
                    )
                });
                if local {
                    return None;
                }
                let text = first.text()?.to_lowercase();
                let (_, import) = imports.iter().find(|(name, _)| *name == text)?;
                Some(Qualified {
                    qualifier,
                    member: second.syntax().text_range(),
                    name: second.text()?,
                    import: import.clone(),
                })
            })
            .collect()
    }

    /// The public member that the qualified name `qualified` reaches, with its module's path.
    fn reached(&self, qualified: &Qualified) -> Option<(String, Member)> {
        let (module, document) = self.imported(&qualified.import)?;
        let member = (module_members(&document.root()).into_iter())
            .find(|member| member.public && member.name.eq_ignore_ascii_case(&qualified.name))?;
        Some((module.to_string(), member))
    }

    /// The member at `offset` in the file at `path`: of a qualified name that holds it, or a
    /// public member whose name holds it in its module's file.
    pub(crate) fn member_at(&self, path: &str, offset: Offset) -> Option<Found> {
        let document = self.document(path)?;
        let offset = document.clamp(offset);
        for qualified in self.qualified_names(document) {
            let part = match () {
                _ if qualified.member.contains_inclusive(offset) => Part::Member,
                _ if qualified.qualifier.contains_inclusive(offset) => Part::Qualifier,
                _ => continue,
            };
            let (module, member) = self.reached(&qualified)?;
            return Some(Found {
                module,
                member,
                at: Some((qualified, part)),
            });
        }
        if !matches!(self.role(path), Some(FileRole::Module { .. })) {
            return None;
        }
        let member = (module_members(&document.root()).into_iter())
            .find(|member| member.public && member.range.contains_inclusive(offset))?;
        Some(Found {
            module: path.to_string(),
            member,
            at: None,
        })
    }

    /// The link of a qualified name at `offset` in the file at `path`: the member's name to its
    /// name in its module, and the qualifier to its import.
    pub(crate) fn member_definition(&self, path: &str, offset: Offset) -> Option<ProjectLink> {
        let found = self.member_at(path, offset)?;
        let (qualified, part) = found.at?;
        Some(match part {
            Part::Member => ProjectLink {
                origin: qualified.member,
                path: found.module,
                target: found.member.range,
            },
            Part::Qualifier => {
                let import = &qualified.import;
                let target = (import.alias().and_then(|alias| alias.name()))
                    .map(|name| trimmed(name.syntax()))
                    .or_else(|| import.path().map(|path| trimmed(path.syntax())))
                    .unwrap_or_else(|| trimmed(import.syntax()));
                ProjectLink {
                    origin: qualified.qualifier,
                    path: path.to_string(),
                    target,
                }
            }
        })
    }

    /// Markdown on the name at `offset` in the file at `path`: a qualified name's member, its
    /// header, its module, and its leading comment, or its qualifier's module, and otherwise
    /// what [`Document::hover`] gives.
    pub fn hover(&self, path: &str, offset: Offset) -> Option<Hover> {
        let document = self.document(path)?;
        let Some(found) = self.member_at(path, offset) else {
            return document.hover(offset);
        };
        let Some((qualified, part)) = found.at else {
            return document.hover(offset);
        };
        let module = (self.module_names(&found.module)).unwrap_or_default();
        if part == Part::Qualifier {
            return Some(Hover {
                range: qualified.qualifier,
                markdown: format!("```googlesql\nMODULE {module}\n```\n\n`{}`", found.module),
            });
        }
        let member = &found.member;
        let (header, node) = match &member.statement {
            Statement::Function(create) => (create_header(create), create.syntax().clone()),
            Statement::Constant(create) => {
                let value = (create.expr())
                    .map(|expr| crate::collapse(&expr.syntax().to_string()))
                    .unwrap_or_default();
                (
                    format!("CONSTANT {} = {value}", member.name),
                    create.syntax().clone(),
                )
            }
        };
        let kind = match &member.statement {
            Statement::Function(_) => "A function",
            Statement::Constant(_) => "A constant",
        };
        let mut markdown = format!(
            "```googlesql\n{header}\n```\n\n{kind} of the module `{module}`, in `{}`.",
            found.module
        );
        if let Some(comment) = leading_comment(&node) {
            markdown.push_str(&format!("\n\n{comment}"));
        }
        Some(Hover {
            range: qualified.member,
            markdown,
        })
    }

    /// The path of the module of the file at `path`, as its `MODULE` statement names it.
    fn module_names(&self, path: &str) -> Option<String> {
        match self.role(path)? {
            FileRole::Module { names } => Some(names.join(".")),
            _ => None,
        }
    }

    /// The forms of the function of the call whose arguments hold `offset` in the file at
    /// `path`: of a module's member, as its `CREATE` writes its parameters, and otherwise what
    /// [`Document::signature_help`] gives.
    pub fn signature_help(&self, path: &str, offset: Offset) -> Option<SignatureHelp> {
        let document = self.document(path)?;
        let offset = document.clamp(offset);
        let member = call_at(&document.root(), offset).and_then(|(list, call)| {
            let path = call.children().find_map(ast::Path::cast)?;
            let start = path.syntax().text_range().start();
            let qualified = (self.qualified_names(document).into_iter())
                .find(|qualified| qualified.qualifier.start() == start)?;
            let (_, member) = self.reached(&qualified)?;
            let Statement::Function(create) = &member.statement else {
                return None;
            };
            let name = crate::collapse(&path.syntax().to_string());
            Some(function_help(create, &name, &list, offset))
        });
        member.or_else(|| document.signature_help(offset))
    }

    /// The items that the text takes at `offset` in the file at `path`: what
    /// [`Document::completions`] gives, and after a qualifier of an import and `.`, the public
    /// members of its module.
    pub fn completions(&self, path: &str, offset: Offset) -> Completions {
        let Some(document) = self.document(path) else {
            let empty = TextRange::empty(offset);
            return Completions {
                range: empty,
                replace: empty,
                items: Vec::new(),
            };
        };
        let mut completions = document.completions(offset);
        let root = document.root();
        let Some(dot) = token_before(&root, completions.range.start())
            .filter(|token| token.kind() == SyntaxKind::DOT)
        else {
            return completions;
        };
        let Some(word) = token_before(&root, dot.text_range().start()) else {
            return completions;
        };
        // A qualifier stands alone before its `.`. A column or a range variable of its name keeps
        // its own items, since a call after the `.` reads a member.
        let alone = token_before(&root, word.text_range().start())
            .is_none_or(|before| before.kind() != SyntaxKind::DOT);
        if !alone {
            return completions;
        }
        let text = word.text().trim_matches('`').to_lowercase();
        let Some(import) = imports(&root)
            .into_iter()
            .find(|import| qualifier(import).is_some_and(|name| name.to_lowercase() == text))
        else {
            return completions;
        };
        let Some((_, module)) = self.imported(&import) else {
            return completions;
        };
        let members: Vec<CompletionItem> = (module_members(&module.root()).into_iter())
            .filter(|member| member.public)
            .map(|member| {
                let (kind, detail) = match &member.statement {
                    Statement::Function(create) => {
                        let role = match create.function_kind() {
                            ast::FunctionKind::Scalar => FunctionRole::Scalar,
                            ast::FunctionKind::Aggregate => FunctionRole::Aggregate,
                            ast::FunctionKind::Table => FunctionRole::Table,
                        };
                        (CompletionKind::Function(role), create_header(create))
                    }
                    Statement::Constant(_) => (
                        CompletionKind::Constant,
                        format!("CONSTANT {}", member.name),
                    ),
                };
                CompletionItem {
                    filter: None,
                    label: member.name.clone(),
                    kind,
                    detail: Some(detail),
                    documentation: None,
                    insert: quote(&member.name),
                    sort: member.name.to_lowercase(),
                }
            })
            .collect();
        let labels: Vec<String> = members
            .iter()
            .map(|item| item.label.to_lowercase())
            .collect();
        completions
            .items
            .retain(|item| !labels.contains(&item.label.to_lowercase()));
        completions.items.extend(members);
        completions.items.sort_by(|a, b| a.sort.cmp(&b.sort));
        completions
    }

    /// The names of the member `member` of the module at `module`, by file in the order of the
    /// paths: its name and its uses in its module, and each qualified name of it in the files
    /// that import the module.
    pub(crate) fn member_references(
        &self,
        module: &str,
        member: &Member,
    ) -> BTreeMap<String, Vec<ProjectReference>> {
        let mut found: BTreeMap<String, Vec<ProjectReference>> = BTreeMap::new();
        if let Some(document) = self.document(module) {
            let mut names = vec![ProjectReference {
                path: module.to_string(),
                range: member.range,
                definition: true,
            }];
            names.extend(
                local_uses(document, member)
                    .into_iter()
                    .map(|range| ProjectReference {
                        path: module.to_string(),
                        range,
                        definition: false,
                    }),
            );
            found.insert(module.to_string(), names);
        }
        for path in self.files().map(str::to_string).collect::<Vec<_>>() {
            let Some(document) = self.document(&path) else {
                continue;
            };
            let uses: Vec<ProjectReference> = (self.qualified_names(document).into_iter())
                .filter(|qualified| {
                    self.reached(qualified).is_some_and(|(reached, other)| {
                        reached == module && other.range == member.range
                    })
                })
                .map(|qualified| ProjectReference {
                    path: path.clone(),
                    range: qualified.member,
                    definition: false,
                })
                .collect();
            if !uses.is_empty() {
                found.entry(path).or_default().extend(uses);
            }
        }
        for references in found.values_mut() {
            references.sort_by_key(|reference| reference.range.start());
            references.dedup_by_key(|reference| reference.range);
        }
        found
    }

    /// The range of a member's name or of a qualifier at `offset` in the file at `path`, which a
    /// rename changes.
    pub(crate) fn member_rename_range(&self, path: &str, offset: Offset) -> Option<TextRange> {
        let found = self.member_at(path, offset)?;
        Some(match found.at {
            Some((qualified, Part::Member)) => qualified.member,
            Some((qualified, Part::Qualifier)) => qualified.qualifier,
            None => found.member.range,
        })
    }

    /// The edits that rename the member or the qualifier at `offset` in the file at `path` to
    /// `name`, by file: a member's name in its module, its uses there, and each qualified name
    /// of it, or a qualifier's alias in its import, added after the module's path when the
    /// import has none, and its uses in its file. A name that another member of the module has
    /// is an error.
    pub(crate) fn member_edits(
        &self,
        path: &str,
        offset: Offset,
        name: &str,
    ) -> Option<Result<BTreeMap<String, Vec<TextEdit>>, String>> {
        let found = self.member_at(path, offset)?;
        Some(self.rename_found(path, found, name))
    }

    fn rename_found(
        &self,
        path: &str,
        found: Found,
        name: &str,
    ) -> Result<BTreeMap<String, Vec<TextEdit>>, String> {
        check_name(name)?;
        let replacement = quote(name);
        if let Some((qualified, Part::Qualifier)) = &found.at {
            let document = self.document(path).expect("the file of the qualifier");
            let import = &qualified.import;
            let mut edits = match import.alias().and_then(|alias| alias.name()) {
                Some(alias) => vec![TextEdit::new(trimmed(alias.syntax()), replacement.clone())],
                None => {
                    let path = import.path().ok_or("The import names no module")?;
                    let end = trimmed(path.syntax()).end();
                    vec![TextEdit::new(
                        TextRange::empty(end),
                        format!(" AS {replacement}"),
                    )]
                }
            };
            let start = import.syntax().text_range().start();
            edits.extend(
                (self.qualified_names(document).into_iter())
                    .filter(|other| other.import.syntax().text_range().start() == start)
                    .map(|other| TextEdit::new(other.qualifier, replacement.clone())),
            );
            return Ok(BTreeMap::from([(path.to_string(), edits)]));
        }
        let module = self.document(&found.module).expect("the module's file");
        let collision = (module_members(&module.root()).into_iter()).find(|other| {
            other.range != found.member.range && other.name.eq_ignore_ascii_case(name)
        });
        if let Some(other) = collision {
            let kind = match other.statement {
                Statement::Function(_) => "function",
                Statement::Constant(_) => "constant",
            };
            return Err(format!(
                "The module `{}` has a {kind} `{}` already",
                self.module_names(&found.module).unwrap_or_default(),
                other.name
            ));
        }
        Ok((self
            .member_references(&found.module, &found.member)
            .into_iter())
        .map(|(path, references)| {
            let edits = (references.into_iter())
                .map(|reference| TextEdit::new(reference.range, replacement.clone()))
                .collect();
            (path, edits)
        })
        .collect())
    }
}
