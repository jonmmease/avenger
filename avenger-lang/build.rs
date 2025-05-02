use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::io::Write;

use check_keyword::CheckKeyword;
use heck::{ToSnekCase, ToUpperCamelCase};


fn main() {
    // Generate typed AST from the tree-sitter grammar
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    generate(
        &tree_sitter_avenger::LANGUAGE.into(),
        tree_sitter_avenger::NODE_TYPES,
        std::fs::File::create(out_dir.join("ast.rs")).unwrap(),
    );
}


/// Generation logic forked from ts-typed-ast

#[derive(Debug, serde::Deserialize)]
struct Node<'a> {
    #[serde(rename = "type", borrow)]
    type_: Cow<'a, str>,
    named: bool,
    #[serde(default)]
    subtypes: Vec<Subtype<'a>>,
    #[serde(default)]
    fields: HashMap<&'a str, Field<'a>>,
    #[allow(dead_code)]
    #[serde(default)]
    children: Option<Field<'a>>,
}

#[derive(Debug, serde::Deserialize)]
struct Subtype<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    named: bool,
}

#[derive(Debug, serde::Deserialize)]
struct Field<'a> {
    multiple: bool,
    required: bool,
    #[serde(borrow)]
    types: Vec<FieldType<'a>>,
}

#[derive(Debug, serde::Deserialize)]
struct FieldType<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    named: bool,
}

pub fn generate(language: &tree_sitter::Language, node_types: &str, mut out_file: impl Write) {
    let mut nodes: VecDeque<Node<'_>> = serde_json::from_str(node_types).unwrap();

    while let Some(Node {
        type_: node_type,
        named,
        subtypes,
        fields,
        children: _,
    }) = nodes.pop_front()
    {
        let node_name = node_type.to_upper_camel_case();

        if !named {
            continue;
        } else if !subtypes.is_empty() {
            writeln!(
                out_file,
                "#[derive(Debug, Clone, Copy)]
pub enum {node_name}<'tree> {{"
            )
            .unwrap();
            for Subtype { type_, named } in &subtypes {
                if !named {
                    continue;
                }
                assert!(
                    named,
                    "subtype '{type_}' of node '{node_type}' is not named"
                );
                let name = type_.to_upper_camel_case();
                writeln!(out_file, "    {name}({name}<'tree>),").unwrap();
            }
            writeln!(out_file, "}}\n").unwrap();

            let can_casts = subtypes
                .iter()
                .map(|Subtype { type_, .. }| {
                    let name = type_.to_upper_camel_case();
                    format!("{name}::can_cast(kind)")
                })
                .collect::<Vec<_>>()
                .join("\n            || ");

            let casts = subtypes
                .iter()
                .map(|Subtype { type_, .. }| {
                    let name = type_.to_upper_camel_case();
                    format!(".or_else(|| {name}::cast(node).map(Self::{name}))")
                })
                .collect::<Vec<_>>()
                .join("\n            ");

            let nodes = subtypes
                .iter()
                .map(|Subtype { type_, .. }| {
                    let name = type_.to_upper_camel_case();
                    format!("Self::{name}(it) => it.node(),")
                })
                .collect::<Vec<_>>()
                .join("\n            ");

            writeln!(
                out_file,
                r#"impl<'tree> crate::tree_sitter_ast::ts_typed_ast::AstNode<'tree> for {node_name}<'tree> {{
    fn can_cast(kind: u16) -> bool {{
        {can_casts}
    }}

    fn cast(node: ::tree_sitter::Node<'tree>) -> Option<Self> {{
        None{casts}
    }}

    fn node(&self) -> ::tree_sitter::Node<'tree> {{
        match self {{
            {nodes}
        }}
    }}
}}
"#
            )
            .unwrap();

            let method_name = node_type.to_snek_case().into_safe();
            writeln!(
                out_file,
                r#"pub trait {node_name}Visitor<'tree> {{
    type Output;

    fn {method_name}(&mut self, {method_name}: {node_name}<'tree>) -> Self::Output {{
        match {method_name} {{"#
            )
            .unwrap();

            for Subtype { type_, .. } in &subtypes {
                let method = type_.into_safe();
                let type_ = type_.to_upper_camel_case();
                writeln!(
                    out_file,
                    "            {node_name}::{type_}({method}) => self.{method}({method}),"
                )
                .unwrap();
            }

            writeln!(
                out_file,
                r#"        }}
    }}
"#
            )
            .unwrap();

            for Subtype { type_, .. } in &subtypes {
                let method = type_.into_safe();
                let type_ = type_.to_upper_camel_case();
                writeln!(
                    out_file,
                    "    fn {method}(&mut self, {method}: {type_}<'tree>) -> Self::Output;"
                )
                .unwrap();
            }

            writeln!(
                out_file,
                r#"}}
"#
            )
            .unwrap();
        } else {
            let kind_id = language.id_for_node_kind(node_type.as_ref(), named);
            writeln!(
                out_file,
                r#"#[derive(Debug, Clone, Copy)]
pub struct {node_name}<'tree> {{
    node: ::tree_sitter::Node<'tree>
}}
"#
            )
            .unwrap();

            writeln!(
                out_file,
                r#"impl<'tree> crate::tree_sitter_ast::ts_typed_ast::AstNode<'tree> for {node_name}<'tree> {{
    fn can_cast(kind: u16) -> bool {{
        kind == {kind_id}
    }}

    fn cast(node: ::tree_sitter::Node<'tree>) -> Option<Self> {{
        Self::can_cast(node.kind_id()).then_some(Self {{ node }})
    }}

    fn node(&self) -> ::tree_sitter::Node<'tree> {{
        self.node
    }}
}}
"#
            )
            .unwrap();

            if !fields.is_empty() {
                writeln!(out_file, "impl<'tree> {node_name}<'tree> {{").unwrap();

                let mut first = true;
                for (
                    name,
                    Field {
                        multiple,
                        required,
                        types,
                    },
                ) in fields
                {
                    // filter out unnamed types
                    let types = types.into_iter().filter(
                        |t| t.named
                    ).collect::<Vec<_>>();
                    if types.is_empty() {
                        continue;
                    }
                    if !first {
                        writeln!(out_file).unwrap();
                    } else {
                        first = false;
                    }
                    let type_ = if types.len() > 1 {
                        let type_ = format!("{node_name}_{name}");
                        nodes.push_front(Node {
                            type_: type_.clone().into(),
                            named: true,
                            subtypes: types
                                .into_iter()
                                .map(|FieldType { type_, named }| Subtype { type_, named })
                                .collect(),
                            fields: Default::default(),
                            children: Default::default(),
                        });
                        type_.to_upper_camel_case()
                    } else {
                        types[0].type_.to_upper_camel_case()
                    };

                    let field_id = language.field_id_for_name(name).unwrap();
                    let method = name.into_safe();
                    if multiple {
                        writeln!(
                            out_file,
                            r#"    pub fn {method}(&self) -> impl Iterator<Item = {type_}<'tree>> {{
        crate::tree_sitter_ast::ts_typed_ast::Children::new(crate::tree_sitter_ast::ts_typed_ast::AstNode::node(self), std::num::NonZeroU16::new({field_id}).unwrap())
    }}"#
                        )
                        .unwrap();
                    } else if required {
                        writeln!(
                            out_file,
                            r#"    pub fn {method}(&self) -> Result<{type_}<'tree>, crate::tree_sitter_ast::ts_typed_ast::MissingNodeChildError<'tree>> {{
        let node = crate::tree_sitter_ast::ts_typed_ast::AstNode::node(self);
        node
            .child_by_field_id({field_id})
            .and_then(crate::tree_sitter_ast::ts_typed_ast::AstNode::cast)
            .ok_or_else(|| crate::tree_sitter_ast::ts_typed_ast::MissingNodeChildError::new(node, {field_id}))
    }}"#
                        )
                        .unwrap();
                    } else {
                        writeln!(
                            out_file,
                            r#"    pub fn {method}(&self) -> Option<{type_}<'tree>> {{
        crate::tree_sitter_ast::ts_typed_ast::AstNode::node(self)
            .child_by_field_id({field_id})
            .and_then(crate::tree_sitter_ast::ts_typed_ast::AstNode::cast)
    }}"#
                        )
                        .unwrap();
                    }
                }

                writeln!(out_file, "}}\n").unwrap();
            }
        }
    }
}
