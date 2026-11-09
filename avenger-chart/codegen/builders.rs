//! The generation of `builders.rs`: a builder for each message of the package, whose methods set
//! the fields of the message and nothing else.
//!
//! The rules, for message `Foo`, where the `required` and `required_oneof` options of
//! `annotation.proto` mark its required content:
//!
//! - `FooBuilder`, an alias of `Builder<Foo>`, for each message except `ValueRef`, `TableRef`,
//!   and `Table`, which stand in for data, with its setters as an impl of `Builder<Foo>`; the
//!   generic `Builder` has `From<Foo>`, `Build`, and the fragments, and each message gets `Finish`
//!   and `Renumber`
//! - the constructors take the required content as arguments: the required fields in the order of
//!   their numbers, then the member of a required oneof. A message without required content has
//!   `Foo::builder()`, a message with required fields and no required oneof has `Foo::new(..)`,
//!   and every member of a oneof has a constructor that takes the required fields first, such as
//!   `Select::interval(selection, interval)`. A message with a required oneof has only the
//!   constructors of that oneof's members, such as `ConditionalValue::direct_value(predicate,
//!   value)`.
//! - one method for each field and each oneof member that is not required content, named after
//!   it, with a trailing underscore for a Rust keyword
//! - a scalar takes its exact Rust type, a string `impl Into<String>`, bytes `impl Into<Bytes>`,
//!   an enum its Rust enum, a message `impl Into<` its builder `>`, an encoding of a channel
//!   `impl IntoEncoding`, a `TableRef` a `RecordBatch`, and a `ValueRef` `impl IntoValue`; a
//!   repeated field takes `impl IntoIterator` of them and appends
//! - a member whose type has no fields takes no argument, and a member whose type has exactly one
//!   field, a repeated one, takes the items of that field
//! - rule A: a message whose only required content is one field converts from that field's
//!   argument: `&str` and `String` for a string, an array or a `Vec` of items for a repeated
//!   field, the field's builder for a message, and the type of a scalar or an enum
//! - rule B: a message with one oneof and no required field converts from each member whose type
//!   is a message that appears in one member only and is not the message itself, from its builder
//!   and from the message
//! - rule F: a message without required content converts from the type of each field that the
//!   `lift` option marks, from its builder and from the message, with only that field set
//! - a type that two rules would convert from converts by none
//! - the docs of each method are a line that says what it does, then the comment of its field
//!
//! `Chart` gets no setters for `version`, `tables`, and `values`, which its `build` writes, and
//! `Plot` gets no `Finish`, because its builder merges the plots that it overlays; the
//! hand-written `compose.rs` adds their methods.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, FieldDescriptorProto, FileDescriptorSet,
};

use super::{annotations::Annotations, camel, ident, snake, Messages, PACKAGE};

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

/// The Rust path of each channel's value, by the name of its encoding fields.
fn channel_paths(set: &FileDescriptorSet) -> BTreeMap<String, String> {
    (set.file.iter())
        .flat_map(|file| &file.enum_type)
        .filter(|enumeration| enumeration.name() == "Channel")
        .flat_map(|enumeration| &enumeration.value)
        .filter(|value| value.number() != 0)
        .map(|value| {
            let name = value
                .name()
                .strip_prefix("CHANNEL_")
                .unwrap_or(value.name());
            let name = name.to_lowercase();
            let path = format!("crate::proto::Channel::{}", camel(&name));
            (name, path)
        })
        .collect()
}

/// A parameter of a constructor and the call of its setter: the parameter's declaration, or `None`
/// for a member without a value, and the setter's name.
struct Parameter {
    declaration: Option<String>,
    setter: String,
}

impl Parameter {
    fn call(&self) -> String {
        let argument = match &self.declaration {
            Some(declaration) => declaration.split(':').next().expect("a name").to_string(),
            None => String::new(),
        };
        format!(".{}({argument})", self.setter)
    }
}

/// The code of `builders.rs`.
pub(super) fn builders(
    set: &FileDescriptorSet,
    messages: &Messages,
    annotations: &Annotations,
) -> String {
    let comments = Comments::new(set);
    let channels = channel_paths(set);
    let encoding = format!("{PACKAGE}.Encoding");
    // The Rust path of the channel of `field`, when it is an encoding of a channel.
    let channel = |field: &FieldDescriptorProto| -> Option<String> {
        (field.type_name() == encoding)
            .then(|| channels.get(field.name()).cloned())
            .flatten()
    };
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

        // The type: an alias of the generic builder.
        docs(
            &mut code,
            "",
            &format!("A builder of [`{short_name}`]."),
            comments.messages.get(name),
        );
        writeln!(
            code,
            "pub type {builder} = crate::builder::Builder<{path}>;\n"
        )
        .unwrap();

        // The required content: the required fields, in the order of their numbers, and the
        // required oneof, whose member a constructor takes last.
        let mut required: Vec<&FieldDescriptorProto> = (message.field.iter())
            .filter(|field| field.oneof_index.is_none() || field.proto3_optional())
            .filter(|field| annotations.is_required(name, field.name()))
            .collect();
        required.sort_by_key(|field| field.number());
        let required_oneofs: Vec<i32> = (0..message.oneof_decl.len() as i32)
            .filter(|oneof| {
                annotations.is_required_oneof(name, message.oneof_decl[*oneof as usize].name())
            })
            .collect();
        assert!(
            required_oneofs.len() <= 1,
            "`{short_name}` has one required oneof at most"
        );
        let required_oneof = required_oneofs.first().copied();
        let mut parameters: BTreeMap<String, Parameter> = BTreeMap::new();
        // The members of the oneofs that constructors take, by oneof, with their comments.
        let mut members: Vec<(i32, Parameter, Option<&String>, String)> = Vec::new();

        let mut setters = String::new();
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
                !matches!(setter.as_str(), "build" | "builder" | "into_parts" | "new"),
                "the field `{field_name}` of `{short_name}` names a function of its builder"
            );
            let member = field.oneof_index.filter(|_| !field.proto3_optional());
            // Required content has no public setter: the constructors take it.
            let hidden = match member {
                Some(oneof) => Some(oneof) == required_oneof,
                None => required.iter().any(|r| r.name() == field_name),
            };
            let (vis, comment) = match hidden {
                true => ("pub(crate)", None),
                false => ("pub", comment),
            };
            let doc = |setters: &mut String, summary: &str| {
                if !hidden {
                    docs(setters, "    ", summary, comment);
                }
            };
            match member {
                Some(oneof) => {
                    let oneof_name = message.oneof_decl[oneof as usize].name();
                    let oneof_ident = ident(oneof_name);
                    let variant = format!("{module}::{}::{}", camel(oneof_name), camel(field_name));
                    let summary = format!(
                        "Sets the `{oneof_name}` oneof to `{field_name}`, replacing any other \
                         member."
                    );
                    let boxed = field.type_name() == name.as_str();
                    let wrap = |value: &str| match boxed {
                        true => format!("Box::new({value})"),
                        false => value.to_string(),
                    };
                    let declaration = match kind(field) {
                        Kind::Scalar(argument, stored) => {
                            doc(&mut setters, &summary);
                            writeln!(
                                setters,
                                "    {vis} fn {setter}(mut self, value: {argument}) -> Self {{\n        self.message.{oneof_ident} = Some({variant}({stored}));\n        self\n    }}\n"
                            )
                            .unwrap();
                            Some(format!("{field_ident}: {argument}"))
                        }
                        Kind::Value => {
                            doc(&mut setters, &summary);
                            writeln!(
                                setters,
                                "    {vis} fn {setter}(mut self, value: impl crate::builder::IntoValue) -> Self {{\n        let value = self.parts.value(\"{field_name}\", value);\n        self.message.{oneof_ident} = Some({variant}(value));\n        self\n    }}\n"
                            )
                            .unwrap();
                            Some(format!("{field_ident}: impl crate::builder::IntoValue"))
                        }
                        Kind::Table => {
                            doc(&mut setters, &summary);
                            writeln!(
                                setters,
                                "    {vis} fn {setter}(mut self, value: ::arrow::array::RecordBatch) -> Self {{\n        let value = self.parts.table(\"{field_name}\", value);\n        self.message.{oneof_ident} = Some({variant}(value));\n        self\n    }}\n"
                            )
                            .unwrap();
                            Some(format!("{field_ident}: ::arrow::array::RecordBatch"))
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
                                doc(&mut setters, &summary);
                                writeln!(
                                    setters,
                                    "    {vis} fn {setter}(self) -> Self {{\n        self.with_{field_name}({member_builder}::default())\n    }}\n"
                                )
                                .unwrap();
                                None
                            } else if let Some(list) = list_field(&type_name) {
                                let item = item_argument(&list, messages);
                                let list_setter = method(list.name());
                                doc(&mut setters, &summary);
                                let list_builder = match annotations
                                    .is_required(&type_name, list.name())
                                {
                                    true => format!("{}::new(items)", rust_path(&type_name)),
                                    false => {
                                        format!("{member_builder}::default().{list_setter}(items)")
                                    }
                                };
                                writeln!(
                                    setters,
                                    "    {vis} fn {setter}(self, items: impl IntoIterator<Item = {item}>) -> Self {{\n        self.with_{field_name}({list_builder})\n    }}\n"
                                )
                                .unwrap();
                                Some(format!("{field_ident}: impl IntoIterator<Item = {item}>"))
                            } else {
                                let (argument, convert) = match channel(field) {
                                    Some(channel) => (
                                        "impl crate::builder::IntoEncoding".to_string(),
                                        format!("value.into_encoding({channel})"),
                                    ),
                                    None => (
                                        format!("impl Into<{member_builder}>"),
                                        "value.into()".to_string(),
                                    ),
                                };
                                doc(&mut setters, &summary);
                                writeln!(
                                    setters,
                                    "    {vis} fn {setter}(self, value: {argument}) -> Self {{\n        self.with_{field_name}({convert})\n    }}\n"
                                )
                                .unwrap();
                                Some(format!("{field_ident}: {argument}"))
                            }
                        }
                    };
                    members.push((
                        oneof,
                        Parameter {
                            declaration,
                            setter: setter.clone(),
                        },
                        comments.fields.get(&(name.clone(), index)),
                        field_name.to_string(),
                    ));
                }
                None if field.label() == Label::Repeated => {
                    let summary = format!("Adds to `{field_name}`, in order.");
                    doc(&mut setters, &summary);
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
                        "    {vis} fn {setter}(mut self, values: impl IntoIterator<Item = {item}>) -> Self {{\n        for value in values {{\n            {push}\n        }}\n        self\n    }}\n"
                    )
                    .unwrap();
                    parameters.insert(
                        field_name.to_string(),
                        Parameter {
                            declaration: Some(format!(
                                "{field_ident}: impl IntoIterator<Item = {item}>"
                            )),
                            setter: setter.clone(),
                        },
                    );
                }
                None => {
                    let summary = format!("Sets `{field_name}`.");
                    doc(&mut setters, &summary);
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
                        Kind::Message(type_name) => {
                            let (argument, convert) = match channel(field) {
                                Some(channel) => (
                                    "impl crate::builder::IntoEncoding".to_string(),
                                    format!("value.into_encoding({channel})"),
                                ),
                                None => (
                                    format!("impl Into<{}Builder>", rust_path(&type_name)),
                                    "value.into()".to_string(),
                                ),
                            };
                            (
                                argument,
                                format!(
                                    "let value = self.parts.embed(\"{field_name}\", {convert});\n        self.message.{field_ident} = Some(value);"
                                ),
                            )
                        }
                    };
                    writeln!(
                        setters,
                        "    {vis} fn {setter}(mut self, value: {argument}) -> Self {{\n        {body}\n        self\n    }}\n"
                    )
                    .unwrap();
                    parameters.insert(
                        field_name.to_string(),
                        Parameter {
                            declaration: Some(format!("{field_ident}: {argument}")),
                            setter: setter.clone(),
                        },
                    );
                }
            }
        }

        // The message's functions: `builder`, or `new` with the required fields, and the
        // constructors of the members of its oneofs, which take the required fields first.
        let required_parameters: Vec<&Parameter> = (required.iter())
            .map(|field| &parameters[field.name()])
            .collect();
        let prefix: Vec<String> = (required_parameters.iter())
            .filter_map(|parameter| parameter.declaration.clone())
            .collect();
        let prefix_calls: String = required_parameters.iter().map(|p| p.call()).collect();
        let required_names: Vec<String> = (required.iter())
            .map(|field| format!("`{}`", field.name()))
            .collect();
        writeln!(code, "impl {path} {{").unwrap();
        match (required_oneof, required.is_empty()) {
            (Some(_), _) => {}
            (None, true) => writeln!(
                code,
                "    /// A builder of [`{short_name}`] that sets no field.\n    pub fn builder() -> {builder} {{\n        {builder}::default()\n    }}\n"
            )
            .unwrap(),
            (None, false) => {
                writeln!(
                    code,
                    "    /// A builder of [`{short_name}`] with its required fields, {}.",
                    required_names.join(", ")
                )
                .unwrap();
                for field in &required {
                    let index = message.field.iter().position(|f| f.name() == field.name());
                    if let Some(comment) =
                        index.and_then(|index| comments.fields.get(&(name.clone(), index)))
                    {
                        code.push_str("    ///\n");
                        let mut first = true;
                        for line in comment.trim_end().lines() {
                            let line = escape(line.strip_prefix(' ').unwrap_or(line));
                            match first {
                                true => writeln!(code, "    /// `{}`: {line}", field.name()),
                                false => writeln!(code, "    /// {line}"),
                            }
                            .unwrap();
                            first = false;
                        }
                    }
                }
                writeln!(
                    code,
                    "    #[allow(clippy::new_ret_no_self)]\n    pub fn new({}) -> {builder} {{\n        {builder}::default(){prefix_calls}\n    }}\n",
                    prefix.join(", ")
                )
                .unwrap();
            }
        }
        for (oneof, parameter, comment, field_name) in &members {
            if required_oneof.is_some_and(|required| required != *oneof) {
                continue;
            }
            let summary = match required.is_empty() {
                true => format!("A builder of [`{short_name}`] with `{field_name}` set."),
                false => format!(
                    "A builder of [`{short_name}`] with its required fields, {}, and `{field_name}` set.",
                    required_names.join(", ")
                ),
            };
            docs(&mut code, "    ", &summary, *comment);
            let mut declarations = prefix.clone();
            declarations.extend(parameter.declaration.clone());
            writeln!(
                code,
                "    pub fn {}({}) -> {builder} {{\n        {builder}::default(){prefix_calls}{}\n    }}\n",
                parameter.setter,
                declarations.join(", "),
                parameter.call()
            )
            .unwrap();
        }
        code.push_str("}\n\n");

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

        // `compose.rs` finishes a plot, which merges the plots that it overlays.
        if !is_plot {
            writeln!(code, "impl crate::builder::Finish for {path} {{}}\n").unwrap();
        }

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

        let content = Content {
            required: &required,
            required_oneof,
            channel: required.first().and_then(|field| channel(field)),
        };
        conversions(
            &mut code,
            name,
            message,
            &builder,
            messages,
            annotations,
            &content,
        );
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

/// The required content of a message, which decides its conversions.
struct Content<'a> {
    required: &'a [&'a FieldDescriptorProto],
    required_oneof: Option<i32>,
    /// For a message whose one required field is an encoding of a channel, the channel.
    channel: Option<String>,
}

/// The `From` conversions of a message's builder, which `docs` of the module list as rules A, B,
/// and F. Each source type converts by one rule, and a type that two rules would convert from
/// converts by none.
fn conversions(
    code: &mut String,
    name: &str,
    message: &DescriptorProto,
    builder: &str,
    messages: &Messages,
    annotations: &Annotations,
    content: &Content,
) {
    let path = rust_path(name);
    // Each conversion: its generic parameters, its source type, its summary, and its body.
    let mut found: Vec<(String, String, String, String)> = Vec::new();
    let mut push = |generics: &str, source: String, summary: String, body: String| {
        found.push((generics.to_string(), source, summary, body));
    };

    // Rule A: a message whose only required content is one field converts from that field's
    // argument.
    if let ([field], None) = (content.required, content.required_oneof) {
        let field_name = field.name();
        let summary = format!(
            "Converts to `{}` with `{field_name}` set to `value`.",
            short(name)
        );
        let body = format!("{path}::new(value)");
        match kind(field) {
            _ if field.label() == Label::Repeated => {
                let bound = match kind(field) {
                    Kind::Message(type_name) => format!("Into<{}Builder>", rust_path(&type_name)),
                    Kind::Value => "crate::builder::IntoValue".to_string(),
                    Kind::Scalar(argument, _) if argument == "impl Into<String>" => {
                        "Into<String>".to_string()
                    }
                    _ => String::new(),
                };
                if !bound.is_empty() {
                    push(
                        &format!("<I: {bound}, const N: usize>"),
                        "[I; N]".to_string(),
                        summary.clone(),
                        body.clone(),
                    );
                    push(
                        &format!("<I: {bound}>"),
                        "Vec<I>".to_string(),
                        summary,
                        body,
                    );
                }
            }
            Kind::Scalar(argument, _) if argument == "impl Into<String>" => {
                push("<'a>", "&'a str".to_string(), summary.clone(), body.clone());
                push("", "String".to_string(), summary, body);
            }
            Kind::Scalar(argument, _) if !argument.starts_with("impl") => {
                push("", argument, summary, body);
            }
            Kind::Table => push("", "::arrow::array::RecordBatch".to_string(), summary, body),
            Kind::Message(type_name) => {
                let _ = content.channel;
                push(
                    "",
                    format!("{}Builder", rust_path(&type_name)),
                    summary,
                    body,
                );
            }
            _ => {}
        }
    }

    let unrequired = content.required.is_empty() && content.required_oneof.is_none();
    // Rule B: a message with one oneof and no required field converts from each member whose type
    // is a message that appears in one member only and is not the message itself. A member sets
    // the oneof, so the oneof may be required.
    let oneofs: BTreeSet<i32> = (message.field.iter())
        .filter(|field| !field.proto3_optional())
        .filter_map(|field| field.oneof_index)
        .collect();
    let types: Vec<&str> = (message.field.iter())
        .map(|field| field.type_name())
        .collect();
    let oneof = oneofs.first().copied();
    if content.required.is_empty()
        && oneofs.len() == 1
        && content
            .required_oneof
            .is_none_or(|required| Some(required) == oneof)
    {
        let oneof = oneof.expect("one oneof");
        let oneof_name = message.oneof_decl[oneof as usize].name();
        for field in (message.field.iter()).filter(|field| field.oneof_index == Some(oneof)) {
            let Kind::Message(type_name) = kind(field) else {
                continue;
            };
            if type_name == name
                || types.iter().filter(|other| **other == type_name).count() != 1
                || messages.get(&type_name).is_none()
            {
                continue;
            }
            let member_path = rust_path(&type_name);
            let field_name = field.name();
            let summary = format!(
                "Converts to `{}` with `{field_name}` set, the member of `{oneof_name}` that `value` is.",
                short(name)
            );
            push(
                "",
                format!("{member_path}Builder"),
                summary.clone(),
                format!("Self::default().with_{field_name}(value)"),
            );
            push(
                "",
                member_path,
                summary,
                format!("Self::default().with_{field_name}(value.into())"),
            );
        }
    }

    // Rule F: a message without required content converts from the type of each lifted field.
    if unrequired {
        for field in &message.field {
            if !annotations
                .lift
                .contains(&(name.to_string(), field.name().to_string()))
            {
                continue;
            }
            let Kind::Message(type_name) = kind(field) else {
                continue;
            };
            let field_path = rust_path(&type_name);
            let setter = method(field.name());
            let summary = format!(
                "Converts to `{}` with `{}` set to `value`.",
                short(name),
                field.name()
            );
            push(
                "",
                format!("{field_path}Builder"),
                summary.clone(),
                format!("Self::default().{setter}(value)"),
            );
            push(
                "",
                field_path,
                summary,
                format!("Self::default().{setter}(value)"),
            );
        }
    }

    for (index, (generics, source, summary, body)) in found.iter().enumerate() {
        let twice = (found.iter().enumerate())
            .any(|(other, (_, other_source, _, _))| other != index && other_source == source);
        if twice {
            continue;
        }
        writeln!(
            code,
            "impl{generics} From<{source}> for {builder} {{\n    /// {summary}\n    fn from(value: {source}) -> Self {{\n        {body}\n    }}\n}}\n"
        )
        .unwrap();
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
