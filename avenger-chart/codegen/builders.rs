//! The generation of `builders.rs`: a builder for each message of the package, whose methods set
//! the fields of the message and nothing else.
//!
//! The rules, for message `Foo`:
//!
//! - `FooBuilder`, with `Foo::builder()` and `From<Foo> for FooBuilder`, for each message except
//!   `ValueRef`, `TableRef`, and `Table`, which stand in for data
//! - one method for each field and each oneof member, named after it, with a trailing underscore
//!   for a Rust keyword
//! - a scalar takes its exact Rust type, a string `impl Into<String>`, bytes `impl Into<Bytes>`,
//!   an enum its Rust enum, a message `impl Into<` its builder `>`, a `TableRef` a `RecordBatch`,
//!   and a `ValueRef` `impl IntoValue`; a repeated field takes `impl IntoIterator` of them and
//!   appends
//! - each oneof member also gets a constructor on the message, which returns the builder with the
//!   member set; a member whose type has no fields takes no argument, and a member whose type has
//!   exactly one field, a repeated one, takes the items of that field
//! - a message whose only content is one oneof converts from each member whose type is a message
//!   that appears in one member only and is not the message itself
//! - the docs of each method are a line that says what it does, then the comment of its field
//!
//! `Chart` gets no setters for `version`, `tables`, and `values`, which its `build` writes, and
//! `Plot` holds the plots that it overlays; the hand-written `compose.rs` adds their methods.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, FieldDescriptorProto, FileDescriptorSet,
};

use super::{camel, ident, snake, Messages, PACKAGE};

/// The messages without builders, which stand in for data.
const DATA: [&str; 3] = ["ValueRef", "TableRef", "Table"];

/// The fields of `Chart` that its `build` writes.
const CHART_OWNED: [&str; 3] = ["version", "tables", "values"];

/// The comments of the messages and their fields, by the message's full name, and by the message's
/// full name and the field's index.
struct Comments {
    messages: BTreeMap<String, String>,
    fields: BTreeMap<(String, usize), String>,
}

impl Comments {
    fn new(set: &FileDescriptorSet) -> Self {
        let mut messages = BTreeMap::new();
        let mut fields = BTreeMap::new();
        for file in &set.file {
            let Some(info) = &file.source_code_info else {
                continue;
            };
            for location in &info.location {
                let comment = location.leading_comments();
                if comment.is_empty() {
                    continue;
                }
                match location.path.as_slice() {
                    [4, message] => {
                        let name = file.message_type[*message as usize].name();
                        messages.insert(format!("{PACKAGE}.{name}"), comment.to_string());
                    }
                    [4, message, 2, field] => {
                        let name = file.message_type[*message as usize].name();
                        fields.insert(
                            (format!("{PACKAGE}.{name}"), *field as usize),
                            comment.to_string(),
                        );
                    }
                    _ => {}
                }
            }
        }
        Self { messages, fields }
    }
}

/// `comment` as doc lines at `indent`, after the line `summary`. Brackets outside code are escaped,
/// so rustdoc does not read them as links.
fn docs(code: &mut String, indent: &str, summary: &str, comment: Option<&String>) {
    writeln!(code, "{indent}/// {summary}").unwrap();
    let Some(comment) = comment else {
        return;
    };
    writeln!(code, "{indent}///").unwrap();
    let mut fenced = false;
    for line in comment.trim_end().lines() {
        let line = line.strip_prefix(' ').unwrap_or(line);
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            writeln!(code, "{indent}/// {line}").unwrap();
            continue;
        }
        let text = match fenced {
            true => line.to_string(),
            false => escape(line),
        };
        match text.is_empty() {
            true => writeln!(code, "{indent}///").unwrap(),
            false => writeln!(code, "{indent}/// {text}").unwrap(),
        }
    }
}

/// `line` with each bracket outside an inline code span escaped.
fn escape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut code = false;
    for c in line.chars() {
        match c {
            '`' => {
                code = !code;
                out.push(c);
            }
            '[' | ']' if !code => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// The name of the method of a field or a member: its Rust identifier, with a trailing underscore
/// for a keyword.
fn method(name: &str) -> String {
    let ident = ident(name);
    match ident.strip_prefix("r#") {
        Some(keyword) => format!("{keyword}_"),
        None => ident,
    }
}

/// The Rust path of an enum or a message of the package from its full name.
fn rust_path(type_name: &str) -> String {
    let relative = type_name
        .strip_prefix(&format!("{PACKAGE}."))
        .expect("a type of the package");
    let mut parts: Vec<String> = relative.split('.').map(str::to_string).collect();
    let last = parts.pop().expect("a name");
    let modules: Vec<String> = parts.iter().map(|part| snake(part)).collect();
    match modules.is_empty() {
        true => format!("crate::proto::{last}"),
        false => format!("crate::proto::{}::{last}", modules.join("::")),
    }
}

/// The short name of a message of the package from its full name.
fn short(type_name: &str) -> &str {
    type_name.rsplit('.').next().expect("a name")
}

/// The argument of a field or a member of a scalar kind, and the expression that its value
/// `value` becomes in the message, or `None` for a message type.
fn scalar(field: &FieldDescriptorProto) -> Option<(String, String)> {
    Some(match field.r#type() {
        Type::Double => ("f64".into(), "value".into()),
        Type::Float => ("f32".into(), "value".into()),
        Type::Int32 | Type::Sint32 | Type::Sfixed32 => ("i32".into(), "value".into()),
        Type::Int64 | Type::Sint64 | Type::Sfixed64 => ("i64".into(), "value".into()),
        Type::Uint32 | Type::Fixed32 => ("u32".into(), "value".into()),
        Type::Uint64 | Type::Fixed64 => ("u64".into(), "value".into()),
        Type::Bool => ("bool".into(), "value".into()),
        Type::String => ("impl Into<String>".into(), "value.into()".into()),
        Type::Bytes => (
            "impl Into<::prost::bytes::Bytes>".into(),
            "value.into()".into(),
        ),
        Type::Enum => (rust_path(field.type_name()), "value as i32".into()),
        Type::Message | Type::Group => return None,
    })
}

/// The kinds of a field's or a member's value.
enum Kind {
    /// A scalar, a string, bytes, or an enum: the argument and the stored expression of `value`.
    Scalar(String, String),
    /// A `ValueRef`.
    Value,
    /// A `TableRef`.
    Table,
    /// Another message, by its full name.
    Message(String),
}

fn kind(field: &FieldDescriptorProto) -> Kind {
    if let Some((argument, stored)) = scalar(field) {
        return Kind::Scalar(argument, stored);
    }
    match short(field.type_name()) {
        "ValueRef" => Kind::Value,
        "TableRef" => Kind::Table,
        _ => Kind::Message(field.type_name().to_string()),
    }
}

/// The code of `builders.rs`.
pub(super) fn builders(set: &FileDescriptorSet, messages: &Messages) -> String {
    let comments = Comments::new(set);
    let values = messages.holders(&format!("{PACKAGE}.ValueRef"));
    let tables = messages.holders(&format!("{PACKAGE}.TableRef"));
    let fieldless = |type_name: &str| {
        messages
            .get(type_name)
            .is_some_and(|(_, _, message)| message.field.is_empty())
    };
    // The single repeated field of a list wrapper.
    let list_field = |type_name: &str| -> Option<FieldDescriptorProto> {
        let (_, _, message) = messages.get(type_name)?;
        match message.field.as_slice() {
            [field] if field.label() == Label::Repeated => Some(field.clone()),
            _ => None,
        }
    };
    let mut code = String::new();
    for (name, path, message) in &messages.all {
        if DATA.contains(&message.name()) {
            continue;
        }
        let short_name = message.name();
        let builder = format!("{short_name}Builder");
        let module = format!("crate::proto::{}", snake(short_name));
        let is_chart = short_name == "Chart";
        let is_plot = short_name == "Plot";

        // The type.
        docs(
            &mut code,
            "",
            &format!("A builder of [`{short_name}`]."),
            comments.messages.get(name),
        );
        code.push_str("#[derive(Clone, Debug, Default)]\n");
        writeln!(code, "pub struct {builder} {{").unwrap();
        writeln!(code, "    pub(crate) message: {path},").unwrap();
        code.push_str("    pub(crate) parts: crate::builder::Parts,\n");
        if is_plot {
            code.push_str("    pub(crate) overlays: Vec<crate::proto::NodeBuilder>,\n");
        }
        code.push_str("}\n\n");

        let mut setters = String::new();
        let mut constructors = String::new();
        let mut helpers = String::new();
        for (index, field) in message.field.iter().enumerate() {
            if is_chart && CHART_OWNED.contains(&field.name()) {
                continue;
            }
            let comment = comments.fields.get(&(name.clone(), index));
            let field_name = field.name();
            let field_ident = ident(field_name);
            let setter = method(field_name);
            assert!(
                !matches!(setter.as_str(), "build" | "builder" | "into_parts"),
                "the field `{field_name}` of `{short_name}` names a method of its builder"
            );
            let member = field.oneof_index.filter(|_| !field.proto3_optional());
            match member {
                Some(oneof) => {
                    let oneof_name = message.oneof_decl[oneof as usize].name();
                    let oneof_ident = ident(oneof_name);
                    let variant = format!("{module}::{}::{}", camel(oneof_name), camel(field_name));
                    let summary = format!(
                        "Sets the `{oneof_name}` oneof to `{field_name}`, replacing any other \
                         member."
                    );
                    let constructor_summary =
                        format!("A builder of [`{short_name}`] with `{field_name}` set.");
                    let boxed = field.type_name() == name.as_str();
                    let wrap = |value: &str| match boxed {
                        true => format!("Box::new({value})"),
                        false => value.to_string(),
                    };
                    match kind(field) {
                        Kind::Scalar(argument, stored) => {
                            docs(&mut setters, "    ", &summary, comment);
                            writeln!(
                                setters,
                                "    pub fn {setter}(mut self, value: {argument}) -> Self {{\n        self.message.{oneof_ident} = Some({variant}({stored}));\n        self\n    }}\n"
                            )
                            .unwrap();
                            docs(&mut constructors, "    ", &constructor_summary, comment);
                            writeln!(
                                constructors,
                                "    pub fn {setter}(value: {argument}) -> {builder} {{\n        {builder}::default().{setter}(value)\n    }}\n"
                            )
                            .unwrap();
                        }
                        Kind::Value => {
                            docs(&mut setters, "    ", &summary, comment);
                            writeln!(
                                setters,
                                "    pub fn {setter}(mut self, value: impl crate::builder::IntoValue) -> Self {{\n        let value = self.parts.value(\"{field_name}\", value);\n        self.message.{oneof_ident} = Some({variant}(value));\n        self\n    }}\n"
                            )
                            .unwrap();
                            docs(&mut constructors, "    ", &constructor_summary, comment);
                            writeln!(
                                constructors,
                                "    pub fn {setter}(value: impl crate::builder::IntoValue) -> {builder} {{\n        {builder}::default().{setter}(value)\n    }}\n"
                            )
                            .unwrap();
                        }
                        Kind::Table => {
                            docs(&mut setters, "    ", &summary, comment);
                            writeln!(
                                setters,
                                "    pub fn {setter}(mut self, value: ::arrow::array::RecordBatch) -> Self {{\n        let value = self.parts.table(\"{field_name}\", value);\n        self.message.{oneof_ident} = Some({variant}(value));\n        self\n    }}\n"
                            )
                            .unwrap();
                            docs(&mut constructors, "    ", &constructor_summary, comment);
                            writeln!(
                                constructors,
                                "    pub fn {setter}(value: ::arrow::array::RecordBatch) -> {builder} {{\n        {builder}::default().{setter}(value)\n    }}\n"
                            )
                            .unwrap();
                        }
                        Kind::Message(type_name) => {
                            let member_builder = format!("{}Builder", rust_path(&type_name));
                            // The member set from a builder, for the setter, the constructor, and
                            // the conversions of a wrapper.
                            writeln!(
                                helpers,
                                "    pub(crate) fn with_{field_name}(mut self, value: {member_builder}) -> Self {{\n        let value = self.parts.embed(\"{field_name}\", value);\n        self.message.{oneof_ident} = Some({variant}({}));\n        self\n    }}\n",
                                wrap("value")
                            )
                            .unwrap();
                            if fieldless(&type_name) {
                                docs(&mut setters, "    ", &summary, comment);
                                writeln!(
                                    setters,
                                    "    pub fn {setter}(self) -> Self {{\n        self.with_{field_name}({member_builder}::default())\n    }}\n"
                                )
                                .unwrap();
                                docs(&mut constructors, "    ", &constructor_summary, comment);
                                writeln!(
                                    constructors,
                                    "    pub fn {setter}() -> {builder} {{\n        {builder}::default().{setter}()\n    }}\n"
                                )
                                .unwrap();
                            } else if let Some(list) = list_field(&type_name) {
                                let item = item_argument(&list, messages);
                                let list_setter = method(list.name());
                                docs(&mut setters, "    ", &summary, comment);
                                writeln!(
                                    setters,
                                    "    pub fn {setter}(self, items: impl IntoIterator<Item = {item}>) -> Self {{\n        self.with_{field_name}({member_builder}::default().{list_setter}(items))\n    }}\n"
                                )
                                .unwrap();
                                docs(&mut constructors, "    ", &constructor_summary, comment);
                                writeln!(
                                    constructors,
                                    "    pub fn {setter}(items: impl IntoIterator<Item = {item}>) -> {builder} {{\n        {builder}::default().{setter}(items)\n    }}\n"
                                )
                                .unwrap();
                            } else {
                                docs(&mut setters, "    ", &summary, comment);
                                writeln!(
                                    setters,
                                    "    pub fn {setter}(self, value: impl Into<{member_builder}>) -> Self {{\n        self.with_{field_name}(value.into())\n    }}\n"
                                )
                                .unwrap();
                                docs(&mut constructors, "    ", &constructor_summary, comment);
                                writeln!(
                                    constructors,
                                    "    pub fn {setter}(value: impl Into<{member_builder}>) -> {builder} {{\n        {builder}::default().{setter}(value)\n    }}\n"
                                )
                                .unwrap();
                            }
                        }
                    }
                }
                None if field.label() == Label::Repeated => {
                    let summary = format!("Adds to `{field_name}`, in order.");
                    docs(&mut setters, "    ", &summary, comment);
                    let item = item_argument(field, messages);
                    let push = match kind(field) {
                        Kind::Scalar(_, stored) => {
                            format!("self.message.{field_ident}.push({stored});")
                        }
                        Kind::Value => format!(
                            "let path = format!(\"{field_name}[{{}}]\", self.message.{field_ident}.len());\n            let value = self.parts.value(&path, value);\n            self.message.{field_ident}.push(value);"
                        ),
                        Kind::Table => format!(
                            "let path = format!(\"{field_name}[{{}}]\", self.message.{field_ident}.len());\n            let value = self.parts.table(&path, value);\n            self.message.{field_ident}.push(value);"
                        ),
                        Kind::Message(_) => format!(
                            "let path = format!(\"{field_name}[{{}}]\", self.message.{field_ident}.len());\n            let value = self.parts.embed(&path, value.into());\n            self.message.{field_ident}.push(value);"
                        ),
                    };
                    writeln!(
                        setters,
                        "    pub fn {setter}(mut self, values: impl IntoIterator<Item = {item}>) -> Self {{\n        for value in values {{\n            {push}\n        }}\n        self\n    }}\n"
                    )
                    .unwrap();
                }
                None => {
                    let summary = format!("Sets `{field_name}`.");
                    docs(&mut setters, "    ", &summary, comment);
                    let optional = field.proto3_optional();
                    let (argument, body) = match kind(field) {
                        Kind::Scalar(argument, stored) => {
                            let stored = match optional {
                                true => format!("Some({stored})"),
                                false => stored,
                            };
                            (
                                argument,
                                format!("self.message.{field_ident} = {stored};"),
                            )
                        }
                        Kind::Value => (
                            "impl crate::builder::IntoValue".to_string(),
                            format!(
                                "let value = self.parts.value(\"{field_name}\", value);\n        self.message.{field_ident} = Some(value);"
                            ),
                        ),
                        Kind::Table => (
                            "::arrow::array::RecordBatch".to_string(),
                            format!(
                                "let value = self.parts.table(\"{field_name}\", value);\n        self.message.{field_ident} = Some(value);"
                            ),
                        ),
                        Kind::Message(type_name) => (
                            format!("impl Into<{}Builder>", rust_path(&type_name)),
                            format!(
                                "let value = self.parts.embed(\"{field_name}\", value.into());\n        self.message.{field_ident} = Some(value);"
                            ),
                        ),
                    };
                    writeln!(
                        setters,
                        "    pub fn {setter}(mut self, value: {argument}) -> Self {{\n        {body}\n        self\n    }}\n"
                    )
                    .unwrap();
                }
            }
        }

        // The message's functions: `builder` and the constructors.
        writeln!(code, "impl {path} {{").unwrap();
        writeln!(
            code,
            "    /// A builder of [`{short_name}`] that sets no field.\n    pub fn builder() -> {builder} {{\n        {builder}::default()\n    }}\n"
        )
        .unwrap();
        code.push_str(&constructors);
        code.push_str("}\n\n");

        writeln!(
            code,
            "impl From<{path}> for {builder} {{\n    /// A builder of `message` as it is, which holds no tables and no values, so its references refer to none.\n    fn from(message: {path}) -> Self {{\n        Self {{\n            message,\n            ..Default::default()\n        }}\n    }}\n}}\n"
        )
        .unwrap();

        // The builder's methods.
        writeln!(code, "impl {builder} {{").unwrap();
        code.push_str(&setters);
        code.push_str(&helpers);
        if !is_chart {
            writeln!(
                code,
                "    /// The message, when the builder holds no tables, no values, and no charts, which get\n    /// their references only in a chart.\n    pub fn build(self) -> Result<{path}, crate::ChartError> {{\n        let (message, parts) = crate::builder::Build::into_parts(self);\n        parts.standalone()?;\n        Ok(message)\n    }}"
            )
            .unwrap();
        }
        code.push_str("}\n\n");

        let into_parts = match is_plot {
            true => "crate::compose::overlaid(self)".to_string(),
            false => "(self.message, self.parts)".to_string(),
        };
        writeln!(
            code,
            "impl crate::builder::Build for {builder} {{\n    type Message = {path};\n    fn into_parts(self) -> ({path}, crate::builder::Parts) {{\n        {into_parts}\n    }}\n}}\n"
        )
        .unwrap();

        let mut renumber = String::new();
        if values.contains(name) {
            renumber.push_str(
                "        crate::walk::VisitValues::visit_values_mut(self, &mut |value| value.index += values);\n",
            );
        }
        if tables.contains(name) {
            renumber.push_str(
                "        crate::walk::VisitTables::visit_tables_mut(self, &mut |table| table.index += tables);\n",
            );
        }
        if renumber.is_empty() {
            renumber.push_str("        let _ = (values, tables);\n");
        } else if !values.contains(name) {
            renumber.insert_str(0, "        let _ = values;\n");
        } else if !tables.contains(name) {
            renumber.insert_str(0, "        let _ = tables;\n");
        }
        writeln!(
            code,
            "impl crate::builder::Renumber for {path} {{\n    fn renumber(&mut self, values: u32, tables: u32) {{\n{renumber}    }}\n}}\n"
        )
        .unwrap();

        wrapper_conversions(&mut code, name, message, &builder, messages);
    }
    code
}

/// The argument of one item of the repeated field `field`.
fn item_argument(field: &FieldDescriptorProto, messages: &Messages) -> String {
    match kind(field) {
        Kind::Scalar(argument, _) => argument,
        Kind::Value => "impl crate::builder::IntoValue".to_string(),
        Kind::Table => "::arrow::array::RecordBatch".to_string(),
        Kind::Message(type_name) => {
            let _ = messages;
            format!("impl Into<{}Builder>", rust_path(&type_name))
        }
    }
}

/// The conversions of a message whose only content is one oneof from each member whose type is a
/// message that appears in one member only and is not the message itself.
fn wrapper_conversions(
    code: &mut String,
    name: &str,
    message: &DescriptorProto,
    builder: &str,
    messages: &Messages,
) {
    let oneofs: BTreeSet<i32> = (message.field.iter())
        .map(|field| match field.oneof_index {
            Some(oneof) if !field.proto3_optional() => oneof,
            _ => -1,
        })
        .collect();
    if oneofs.len() != 1 || oneofs.contains(&-1) {
        return;
    }
    let types: Vec<&str> = (message.field.iter())
        .map(|field| field.type_name())
        .collect();
    for field in &message.field {
        let Kind::Message(type_name) = kind(field) else {
            continue;
        };
        if type_name == name || types.iter().filter(|other| **other == type_name).count() != 1 {
            continue;
        }
        if messages.get(&type_name).is_none() {
            continue;
        }
        let member_path = rust_path(&type_name);
        let member_builder = format!("{member_path}Builder");
        let field_name = field.name();
        let oneof_name = message.oneof_decl[0].name();
        for (source, value) in [(&member_builder, "value"), (&member_path, "value.into()")] {
            writeln!(
                code,
                "impl From<{source}> for {builder} {{\n    /// Converts to `{}` with `{field_name}` set, the member of `{oneof_name}` that `value` is.\n    fn from(value: {source}) -> Self {{\n        Self::default().with_{field_name}({value})\n    }}\n}}\n",
                short(name)
            )
            .unwrap();
        }
    }
}

/// The code of `prelude.rs`: a `pub use` of every message with a builder except `Chart`, whose
/// name the prelude gives to the checked chart, of every builder, and of every enum of the
/// package.
pub(super) fn prelude(set: &FileDescriptorSet, messages: &Messages) -> String {
    let mut names: Vec<String> = Vec::new();
    for (_, _, message) in &messages.all {
        let name = message.name();
        if DATA.contains(&name) {
            continue;
        }
        if name != "Chart" {
            names.push(name.to_string());
        }
        names.push(format!("{name}Builder"));
    }
    for file in &set.file {
        names.extend(
            file.enum_type
                .iter()
                .map(|enumeration| enumeration.name().to_string()),
        );
    }
    names.sort();
    let mut code = String::from("pub use crate::proto::{\n");
    for name in names {
        writeln!(code, "    {name},").unwrap();
    }
    code.push_str("};\n");
    code
}
