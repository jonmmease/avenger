//! Generates the syntax tree's node kinds and typed nodes from `sql.ungram`.
//!
//! It writes these files to `OUT_DIR`:
//!
//! - `kinds.rs`: the macro `node_kinds!`, which adds the node kinds, in the grammar's order and
//!   with its `///` docs, to the invocation of `syntax_kinds!` in `src/syntax_kind.rs`.
//! - `nodes.rs`: for each rule whose alternatives are all nodes, an enum of those nodes; for
//!   each other rule, a struct around its `SyntaxNode`, with an accessor for each child; and
//!   `shape`, the kinds of the children and tokens that a node of each kind can hold.
//!
//! An accessor of a token returns its first token of the token's kinds. An accessor of a node
//! returns the first child of the node's type, or with a label, as in `rhs:Expr`, the child of
//! that type at the label's place among the labels of the type. An accessor of a node that a
//! rule repeats returns all the children of its type. The script fails when a rule's children
//! are ambiguous: two unlabeled children of one type outside a repetition, or two children of
//! different types that one node can match.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    path::PathBuf,
};

use ungrammar::{Grammar, Rule};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let path = manifest.join("sql.ungram");
    println!("cargo:rerun-if-changed=sql.ungram");
    println!("cargo:rerun-if-changed=build.rs");
    let text = std::fs::read_to_string(&path).unwrap();
    let grammar: Grammar = text
        .parse()
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let model = Model::new(&grammar, &text);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out.join("kinds.rs"), model.kinds()).unwrap();
    std::fs::write(out.join("nodes.rs"), model.nodes()).unwrap();
}

/// A rule of the grammar: an enum of nodes, or a node with fields.
enum Def {
    Enum(Vec<String>),
    Struct(Vec<Field>),
}

#[derive(Clone, Debug)]
enum Field {
    Token {
        name: String,
        kinds: BTreeSet<String>,
    },
    Node {
        name: String,
        labeled: bool,
        ty: String,
        many: bool,
        nth: usize,
    },
}

impl Field {
    fn name(&self) -> &str {
        match self {
            Field::Token { name, .. } | Field::Node { name, .. } => name,
        }
    }
}

struct Model {
    /// The rules in the order of the grammar.
    order: Vec<String>,
    defs: BTreeMap<String, Def>,
    docs: BTreeMap<String, Vec<String>>,
}

impl Model {
    fn new(grammar: &Grammar, text: &str) -> Self {
        let docs = docs(text);
        let mut order = Vec::new();
        let mut defs = BTreeMap::new();
        for node in grammar.iter() {
            let data = &grammar[node];
            order.push(data.name.clone());
            let def = match enum_variants(grammar, &data.rule) {
                Some(variants) => Def::Enum(variants),
                None => {
                    let mut fields = Vec::new();
                    lower(grammar, &data.rule, None, false, &mut fields, &data.name);
                    Def::Struct(fields)
                }
            };
            defs.insert(data.name.clone(), def);
        }
        let mut model = Model { order, defs, docs };
        model.index_fields();
        model
    }

    /// Gives each labeled child its place among the children of its type, and checks that no
    /// two children are ambiguous.
    fn index_fields(&mut self) {
        let kind_sets: BTreeMap<String, BTreeSet<String>> = self
            .order
            .iter()
            .map(|name| (name.clone(), self.kind_set(name)))
            .collect();
        for (rule, def) in self.defs.iter_mut() {
            let Def::Struct(fields) = def else { continue };
            let mut by_type: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for (index, field) in fields.iter().enumerate() {
                if let Field::Node { ty, .. } = field {
                    by_type.entry(ty.clone()).or_default().push(index);
                }
            }
            for (ty, indices) in &by_type {
                if indices.len() > 1 {
                    for (nth, &index) in indices.iter().enumerate() {
                        let Field::Node {
                            labeled,
                            many,
                            name,
                            nth: slot,
                            ..
                        } = &mut fields[index]
                        else {
                            unreachable!()
                        };
                        assert!(
                            *labeled && !*many,
                            "{rule}: the children of type {ty} need labels, as `{name}` has none"
                        );
                        *slot = nth;
                    }
                }
            }
            let types: Vec<&String> = by_type.keys().collect();
            for (i, a) in types.iter().enumerate() {
                for b in &types[i + 1..] {
                    let shared: Vec<_> = kind_sets[*a].intersection(&kind_sets[*b]).collect();
                    assert!(
                        shared.is_empty(),
                        "{rule}: a child can be both {a} and {b}: {shared:?}"
                    );
                }
            }
            let mut names = BTreeSet::new();
            for field in fields.iter() {
                let name = accessor(field);
                assert!(
                    names.insert(name.clone()),
                    "{rule}: two accessors named {name}"
                );
            }
        }
    }

    /// The node kinds that the type `name` matches.
    fn kind_set(&self, name: &str) -> BTreeSet<String> {
        match &self.defs[name] {
            Def::Struct(_) => BTreeSet::from([kind_name(name)]),
            Def::Enum(variants) => variants.iter().flat_map(|v| self.kind_set(v)).collect(),
        }
    }

    fn structs(&self) -> impl Iterator<Item = (&String, &Vec<Field>)> {
        self.order.iter().filter_map(|name| match &self.defs[name] {
            Def::Struct(fields) => Some((name, fields)),
            Def::Enum(_) => None,
        })
    }

    fn kinds(&self) -> String {
        let mut out = String::new();
        writeln!(out, "// Generated by build.rs from sql.ungram.").unwrap();
        writeln!(out, "macro_rules! node_kinds {{").unwrap();
        writeln!(out, "    ($m:ident! {{ $($t:tt)* }}) => {{").unwrap();
        writeln!(out, "        $m! {{ $($t)* nodes: [").unwrap();
        writeln!(
            out,
            "            /// Tokens that the parser skipped while it recovered from an error."
        )
        .unwrap();
        writeln!(out, "            ERROR,").unwrap();
        for (name, _) in self.structs() {
            for line in self.docs.get(name).into_iter().flatten() {
                writeln!(out, "            ///{line}").unwrap();
            }
            writeln!(out, "            {},", kind_name(name)).unwrap();
        }
        writeln!(out, "        ] }}").unwrap();
        writeln!(out, "    }};").unwrap();
        writeln!(out, "}}").unwrap();
        out
    }

    fn nodes(&self) -> String {
        let mut out = String::new();
        writeln!(out, "// Generated by build.rs from sql.ungram.").unwrap();
        for name in &self.order {
            let docs = self.docs.get(name);
            match &self.defs[name] {
                Def::Struct(fields) => self.write_struct(&mut out, name, fields, docs),
                Def::Enum(variants) => self.write_enum(&mut out, name, variants, docs),
            }
        }
        self.write_shape(&mut out);
        out
    }

    fn write_struct(
        &self,
        out: &mut String,
        name: &str,
        fields: &[Field],
        docs: Option<&Vec<String>>,
    ) {
        let kind = kind_name(name);
        write_docs(
            out,
            docs,
            &format!("A node of kind [`SyntaxKind::{kind}`]."),
        );
        writeln!(out, "#[derive(Clone, Debug, PartialEq, Eq, Hash)]").unwrap();
        writeln!(out, "pub struct {name} {{ pub(crate) syntax: SyntaxNode }}").unwrap();
        writeln!(out, "impl AstNode for {name} {{").unwrap();
        writeln!(
            out,
            "    fn can_cast(kind: SyntaxKind) -> bool {{ kind == SyntaxKind::{kind} }}"
        )
        .unwrap();
        writeln!(
            out,
            "    fn cast(syntax: SyntaxNode) -> Option<Self> {{ Self::can_cast(syntax.kind()).then_some(Self {{ syntax }}) }}"
        )
        .unwrap();
        writeln!(
            out,
            "    fn syntax(&self) -> &SyntaxNode {{ &self.syntax }}"
        )
        .unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out, "impl {name} {{").unwrap();
        for field in fields {
            let accessor = accessor(field);
            match field {
                Field::Token { kinds, .. } => {
                    let kinds: Vec<String> =
                        kinds.iter().map(|k| format!("SyntaxKind::{k}")).collect();
                    writeln!(
                        out,
                        "    pub fn {accessor}(&self) -> Option<SyntaxToken> {{ support::token(&self.syntax, &[{}]) }}",
                        kinds.join(", ")
                    )
                    .unwrap();
                }
                Field::Node { ty, many: true, .. } => {
                    writeln!(
                        out,
                        "    pub fn {accessor}(&self) -> AstChildren<{ty}> {{ support::children(&self.syntax) }}"
                    )
                    .unwrap();
                }
                Field::Node { ty, nth, .. } => {
                    writeln!(
                        out,
                        "    pub fn {accessor}(&self) -> Option<{ty}> {{ support::child(&self.syntax, {nth}) }}"
                    )
                    .unwrap();
                }
            }
        }
        writeln!(out, "}}").unwrap();
        write_display(out, name);
    }

    fn write_enum(
        &self,
        out: &mut String,
        name: &str,
        variants: &[String],
        docs: Option<&Vec<String>>,
    ) {
        write_docs(out, docs, &format!("One of: {}.", variants.join(", ")));
        writeln!(out, "#[derive(Clone, Debug, PartialEq, Eq, Hash)]").unwrap();
        writeln!(out, "pub enum {name} {{").unwrap();
        for variant in variants {
            writeln!(out, "    {variant}({variant}),").unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out, "impl AstNode for {name} {{").unwrap();
        let can_cast: Vec<String> = variants
            .iter()
            .map(|variant| format!("{variant}::can_cast(kind)"))
            .collect();
        writeln!(
            out,
            "    fn can_cast(kind: SyntaxKind) -> bool {{ {} }}",
            can_cast.join(" || ")
        )
        .unwrap();
        writeln!(out, "    fn cast(syntax: SyntaxNode) -> Option<Self> {{").unwrap();
        writeln!(out, "        let kind = syntax.kind();").unwrap();
        for variant in variants {
            writeln!(
                out,
                "        if {variant}::can_cast(kind) {{ return {variant}::cast(syntax).map(Self::{variant}); }}"
            )
            .unwrap();
        }
        writeln!(out, "        None").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "    fn syntax(&self) -> &SyntaxNode {{").unwrap();
        writeln!(out, "        match self {{").unwrap();
        for variant in variants {
            writeln!(out, "            Self::{variant}(node) => node.syntax(),").unwrap();
        }
        writeln!(out, "        }}").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        for variant in variants {
            writeln!(
                out,
                "impl From<{variant}> for {name} {{ fn from(node: {variant}) -> Self {{ Self::{variant}(node) }} }}"
            )
            .unwrap();
        }
        write_display(out, name);
    }

    /// `shape`: for each node kind, the kinds of the nodes and the tokens that it can hold.
    fn write_shape(&self, out: &mut String) {
        writeln!(
            out,
            "/// The kinds of the child nodes and of the tokens, other than trivia, that a node of `kind`"
        )
        .unwrap();
        writeln!(
            out,
            "/// can hold, as `sql.ungram` describes it. Every node can also hold `ERROR` nodes."
        )
        .unwrap();
        writeln!(
            out,
            "pub(crate) fn shape(kind: SyntaxKind) -> Option<(&'static [SyntaxKind], &'static [SyntaxKind])> {{"
        )
        .unwrap();
        writeln!(out, "    match kind {{").unwrap();
        for (name, fields) in self.structs() {
            let mut nodes = BTreeSet::new();
            let mut tokens = BTreeSet::new();
            for field in fields {
                match field {
                    Field::Token { kinds, .. } => tokens.extend(kinds.iter().cloned()),
                    Field::Node { ty, .. } => nodes.extend(self.kind_set(ty)),
                }
            }
            let list = |set: &BTreeSet<String>| {
                set.iter()
                    .map(|k| format!("SyntaxKind::{k}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            writeln!(
                out,
                "        SyntaxKind::{} => Some((&[{}], &[{}])),",
                kind_name(name),
                list(&nodes),
                list(&tokens)
            )
            .unwrap();
        }
        writeln!(out, "        _ => None,").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
    }
}

fn write_docs(out: &mut String, docs: Option<&Vec<String>>, default: &str) {
    match docs {
        Some(lines) => {
            for line in lines {
                writeln!(out, "///{line}").unwrap();
            }
        }
        None => writeln!(out, "/// {default}").unwrap(),
    }
}

fn write_display(out: &mut String, name: &str) {
    writeln!(
        out,
        "impl std::fmt::Display for {name} {{ fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ std::fmt::Display::fmt(self.syntax(), f) }} }}"
    )
    .unwrap();
}

/// The variants of a rule whose alternatives are all nodes.
fn enum_variants(grammar: &Grammar, rule: &Rule) -> Option<Vec<String>> {
    let Rule::Alt(alternatives) = rule else {
        return None;
    };
    alternatives
        .iter()
        .map(|alternative| match alternative {
            Rule::Node(node) => Some(grammar[*node].name.clone()),
            _ => None,
        })
        .collect()
}

/// Adds the fields of `rule` to `fields`. A field that two alternatives share, or that a
/// sequence and its repetition share, as in `A (',' A)*`, is one field.
fn lower(
    grammar: &Grammar,
    rule: &Rule,
    label: Option<&str>,
    many: bool,
    fields: &mut Vec<Field>,
    owner: &str,
) {
    match rule {
        Rule::Labeled { label, rule } => lower(grammar, rule, Some(label), many, fields, owner),
        Rule::Node(node) => {
            let ty = grammar[*node].name.clone();
            let name = label.map_or_else(|| snake_name(&ty), str::to_string);
            add_field(
                fields,
                Field::Node {
                    name,
                    labeled: label.is_some(),
                    ty,
                    many,
                    nth: 0,
                },
                owner,
            );
        }
        Rule::Token(token) => {
            let text = &grammar[*token].name;
            let name = label.map_or_else(|| token_name(text), str::to_string);
            add_field(
                fields,
                Field::Token {
                    name,
                    kinds: token_kinds(text),
                },
                owner,
            );
        }
        Rule::Alt(alternatives) if label.is_some() => {
            let mut kinds = BTreeSet::new();
            for alternative in alternatives {
                match alternative {
                    Rule::Token(token) => kinds.extend(token_kinds(&grammar[*token].name)),
                    _ => {
                        // A label on alternatives of nodes labels each of them.
                        lower(grammar, alternative, label, many, fields, owner);
                    }
                }
            }
            if !kinds.is_empty() {
                add_field(
                    fields,
                    Field::Token {
                        name: label.unwrap().to_string(),
                        kinds,
                    },
                    owner,
                );
            }
        }
        Rule::Seq(rules) | Rule::Alt(rules) => {
            for rule in rules {
                lower(grammar, rule, None, many, fields, owner);
            }
        }
        Rule::Opt(rule) => lower(grammar, rule, label, many, fields, owner),
        Rule::Rep(rule) => lower(grammar, rule, label, true, fields, owner),
    }
}

fn add_field(fields: &mut Vec<Field>, field: Field, owner: &str) {
    let existing = fields.iter_mut().find(|f| f.name() == field.name());
    match (existing, field) {
        (None, field) => fields.push(field),
        (Some(Field::Token { kinds, .. }), Field::Token { kinds: more, .. }) => kinds.extend(more),
        (
            Some(Field::Node { ty, many, .. }),
            Field::Node {
                ty: other,
                many: more,
                ..
            },
        ) if *ty == other => *many |= more,
        (Some(existing), field) => panic!(
            "{owner}: the children {existing:?} and {field:?} have one name, {}",
            field.name()
        ),
    }
}

/// The name of a field's accessor: a node's label, or its type's snake name, plural when the
/// rule repeats it.
fn accessor(field: &Field) -> String {
    match field {
        Field::Token { name, .. } => name.clone(),
        Field::Node {
            name,
            labeled,
            many,
            ..
        } => {
            let name = match (*many, *labeled) {
                (true, false) => plural(name),
                _ => name.clone(),
            };
            match name.as_str() {
                "type" => "ty".to_string(),
                _ => name,
            }
        }
    }
}

fn plural(name: &str) -> String {
    if let Some(stem) = name.strip_suffix('y') {
        if !stem.ends_with(['a', 'e', 'o', 'u']) {
            return format!("{stem}ies");
        }
    }
    if name.ends_with('s') || name.ends_with('x') {
        return format!("{name}es");
    }
    format!("{name}s")
}

/// The kind of the rule `name`, such as `PIPE_WHERE` for `PipeWhere`.
fn kind_name(name: &str) -> String {
    snake_name(name).to_ascii_uppercase()
}

fn snake_name(name: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() && previous_lower {
            out.push('_');
        }
        previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// The kinds of a token of the grammar.
fn token_kinds(text: &str) -> BTreeSet<String> {
    let kinds: &[&str] = match text {
        "#ident" => &["WORD", "QUOTED_IDENT"],
        "#string" => &["STRING"],
        "#bytes" => &["BYTES"],
        "#int" => &["INTEGER"],
        "#float" => &["FLOAT"],
        "#param" => &["PARAM"],
        _ => {
            return BTreeSet::from([
                punctuation(text).map_or_else(|| keyword_kind(text), str::to_string)
            ])
        }
    };
    kinds.iter().map(|k| k.to_string()).collect()
}

fn keyword_kind(text: &str) -> String {
    assert!(
        text.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
        "'{text}' is neither punctuation nor a lowercase keyword"
    );
    format!("{}_KW", text.to_ascii_uppercase())
}

fn punctuation(text: &str) -> Option<&'static str> {
    let kind = match text {
        "|>" => "PIPE",
        "(" => "L_PAREN",
        ")" => "R_PAREN",
        "[" => "L_BRACK",
        "]" => "R_BRACK",
        "," => "COMMA",
        "." => "DOT",
        ";" => "SEMICOLON",
        "=" => "EQ",
        "!=" => "NEQ",
        "<" => "LT",
        "<=" => "LT_EQ",
        ">" => "GT",
        ">=" => "GT_EQ",
        "+" => "PLUS",
        "-" => "MINUS",
        "*" => "STAR",
        "/" => "SLASH",
        "||" => "CONCAT",
        "|" => "BIT_OR",
        "&" => "AMP",
        "^" => "CARET",
        "~" => "TILDE",
        "<<" => "SHL",
        "=>" => "FAT_ARROW",
        "?" => "POSITIONAL",
        "$" => "DOLLAR",
        "{" => "L_BRACE",
        "}" => "R_BRACE",
        _ => return None,
    };
    Some(kind)
}

/// The name of a token's accessor: `where_kw` for 'where', `l_paren` for '('.
fn token_name(text: &str) -> String {
    match text.strip_prefix('#') {
        Some(class) => class.to_string(),
        None => match punctuation(text) {
            Some(kind) => kind.to_ascii_lowercase(),
            None => format!("{text}_kw"),
        },
    }
}

/// The `///` docs of each rule: the doc lines right above the line that starts it.
fn docs(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut docs = BTreeMap::new();
    let mut pending = Vec::new();
    for line in text.lines() {
        if let Some(doc) = line.strip_prefix("///") {
            pending.push(doc.to_string());
            continue;
        }
        if let Some((name, _)) = line.split_once(" =") {
            if !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric())
                && !pending.is_empty()
            {
                docs.insert(name.to_string(), std::mem::take(&mut pending));
            }
        }
        if !line.starts_with("//") {
            pending.clear();
        }
    }
    docs
}
