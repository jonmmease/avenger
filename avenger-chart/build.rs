//! Generates the prost messages of package `avenger.chart.v1` from the files in
//! `proto/avenger/chart/v1/` with the vendored `protoc`, so that a build needs no system `protoc`.
//!
//! It also reads the descriptors of the messages and generates two files:
//!
//! - `walk.rs`: impls of `VisitValues` for each message that holds a `ValueRef`, of `VisitTables`
//!   for each message that holds a `TableRef`, and of `VisitEnums` for each message that holds an
//!   enum field, directly or through its fields, and `enum_error`, which checks the value of an
//!   enum field: a value that its enum declares, and not the zero value of an enum whose zero
//!   value's comment says that it is not valid.
//! - `marks.rs`: for each mark, the member of the `mark` oneof of a layer, its channels and their
//!   encodings, and for each `mark` oneof, an impl of `MarkMessage`.
//! - `merge.rs`: an impl of `MergeFields` for each message that the specs of a shared scale merge
//!   field by field: the guides, the options of the scale types, and the fits.
//!
//! It fails when a field of a mark breaks a law of the format: an `Encoding` field has the name of
//! a `Channel` value, lowercase and without the prefix, and the number 100 plus that value, and
//! every other field, a property, has a number below 100.

use std::{collections::BTreeSet, fmt::Write, path::PathBuf};

use prost::Message;
use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorSet,
};

const PROTOS: [&str; 25] = [
    "annotation",
    "attribution",
    "axis",
    "cartesian",
    "channel",
    "chart",
    "encoding",
    "facet",
    "geo",
    "guide",
    "guide_element",
    "tool",
    "legend",
    "mark",
    "option",
    "partition",
    "plot",
    "polar",
    "resolve",
    "route",
    "scale",
    "selection",
    "side",
    "style",
    "value",
];

const PACKAGE: &str = ".avenger.chart.v1";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptors = out.join("descriptors.bin");
    let files = PROTOS.map(|name| format!("proto/avenger/chart/v1/{name}.proto"));
    prost_build::Config::new()
        .protoc_executable(protoc_bin_vendored::protoc_bin_path()?)
        .file_descriptor_set_path(&descriptors)
        // Every `bytes` field is a `bytes::Bytes`, which decodes as a slice of its input, so the
        // Arrow payloads of a chart decode without a copy.
        .bytes(["."])
        .compile_protos(
            &files,
            &[PathBuf::from("proto"), protoc_bin_vendored::include_path()?],
        )?;

    let mut set = FileDescriptorSet::decode(std::fs::read(&descriptors)?.as_slice())?;
    // The set also holds the files that the package imports, such as `descriptor.proto`.
    set.file
        .retain(|file| format!(".{}", file.package()) == PACKAGE);
    let messages = Messages::new(&set);
    let mut walk = String::new();
    for (target, visitor, method) in [
        ("ValueRef", "VisitValues", "visit_values"),
        ("TableRef", "VisitTables", "visit_tables"),
    ] {
        messages.visitors(&mut walk, &format!("{PACKAGE}.{target}"), visitor, method);
    }
    messages.enum_visitors(&mut walk);
    let bytes = std::fs::read(&descriptors)?;
    enum_error(&mut walk, &set, &invalid_values(&bytes)?);
    std::fs::write(out.join("walk.rs"), walk)?;
    std::fs::write(out.join("marks.rs"), marks(&set, &messages)?)?;
    std::fs::write(out.join("merge.rs"), messages.merges())?;
    Ok(())
}

/// The kinds of coordinate systems, by the prefix of the names of their layer messages.
const KINDS: [(&str, &str); 3] = [
    ("Cartesian", "Cartesian"),
    ("Polar", "Polar"),
    ("Geo", "Geo"),
];

/// How a mark holds the encoding of a channel: in a field of its own, or as a member of a oneof
/// of the mark, as `PolarTextMark.turn` holds `angle`.
enum Holder {
    Field(String),
    Oneof { field: String, path: String },
}

/// The code of `marks.rs`, after a check of the laws of the fields of each mark.
fn marks(set: &FileDescriptorSet, messages: &Messages) -> Result<String, String> {
    let channels: Vec<(String, i32)> = (set.file.iter())
        .flat_map(|file| &file.enum_type)
        .find(|enumeration| enumeration.name() == "Channel")
        .ok_or("the package has a `Channel` enum")?
        .value
        .iter()
        .filter(|value| value.number() != 0)
        .map(|value| {
            let name = value
                .name()
                .strip_prefix("CHANNEL_")
                .unwrap_or(value.name());
            (name.to_lowercase(), value.number())
        })
        .collect();
    // The layers: the messages with a `mark` oneof.
    let layers: Vec<(&String, &String, &DescriptorProto, usize)> = (messages.all.iter())
        .filter_map(|(name, path, message)| {
            let oneof = (message.oneof_decl.iter()).position(|oneof| oneof.name() == "mark")?;
            Some((name, path, *message, oneof))
        })
        .collect();
    if layers.is_empty() {
        return Err("the package has layers, messages with a `mark` oneof".to_string());
    }

    let mut code = String::new();
    for (_, layer_path, layer, oneof) in layers {
        let &(_, kind) = KINDS
            .iter()
            .find(|(prefix, _)| layer.name().starts_with(prefix))
            .ok_or_else(|| format!("`{}` names its coordinate system", layer.name()))?;
        let module = format!(
            "{}::{}",
            layer_path.rsplit_once("::").expect("a nested path").0,
            snake(layer.name())
        );
        let members: Vec<&FieldDescriptorProto> = (layer.field.iter())
            .filter(|field| field.oneof_index == Some(oneof as i32))
            .collect();
        let mut arms = Vec::new();
        for member in members {
            let (_, path, mark) = messages
                .get(member.type_name())
                .ok_or_else(|| format!("`{}.{}` is a message", layer.name(), member.name()))?;
            let mut taken: Vec<(String, Holder)> = Vec::new();
            for field in &mark.field {
                let at = format!("`{}.{}`", mark.name(), field.name());
                if field.r#type() == Type::Message
                    && field.type_name() == format!("{PACKAGE}.Encoding")
                {
                    let (name, value) = (channels.iter())
                        .find(|(name, _)| name == field.name())
                        .ok_or_else(|| {
                            format!("the encoding field {at} has the name of a channel")
                        })?;
                    if field.number() != 100 + value {
                        return Err(format!(
                            "the encoding field {at} has the number {}, 100 plus its channel's value",
                            100 + value
                        ));
                    }
                    let holder = match field.oneof_index {
                        Some(index) if !field.proto3_optional() => {
                            let oneof = mark.oneof_decl[index as usize].name();
                            Holder::Oneof {
                                field: ident(oneof),
                                path: format!(
                                    "{}::{}::{}",
                                    path.rsplit_once("::").expect("a nested path").0,
                                    snake(mark.name()),
                                    camel(oneof)
                                ),
                            }
                        }
                        _ => Holder::Field(ident(field.name())),
                    };
                    taken.push((name.clone(), holder));
                } else if field.number() >= 100 {
                    return Err(format!("the property {at} has a number below 100"));
                }
            }
            write_mark(&mut code, path, &taken);
            let variant = camel(member.name());
            writeln!(
                code,
                "impl From<{path}> for {module}::Mark {{\n    fn from(mark: {path}) -> Self {{\n        Self::{variant}(mark)\n    }}\n}}\n"
            )
            .unwrap();
            arms.push((variant, member.name().to_string(), path.clone()));
        }

        // One match over the members of the `mark` oneof for each method of `MarkMessage`.
        let method =
            |code: &mut String, signature: &str, arm: &dyn Fn(&str, &str, &str) -> String| {
                writeln!(code, "    fn {signature} {{\n        match self {{").unwrap();
                for (variant, name, path) in &arms {
                    writeln!(code, "            {},", arm(variant, name, path)).unwrap();
                }
                code.push_str("        }\n    }\n");
            };
        writeln!(code, "impl MarkMessage for {module}::Mark {{").unwrap();
        writeln!(
            code,
            "    fn kind(&self) -> CoordKind {{\n        CoordKind::{kind}\n    }}"
        )
        .unwrap();
        method(
            &mut code,
            "mark_type(&self) -> MarkType",
            &|variant, _, _| format!("Self::{variant}(_) => MarkType::{variant}"),
        );
        method(
            &mut code,
            "name(&self) -> &'static str",
            &|variant, name, _| format!("Self::{variant}(_) => \"{name}\""),
        );
        method(
            &mut code,
            "channels(&self) -> &'static [Channel]",
            &|variant, _, path| format!("Self::{variant}(_) => {path}::CHANNELS"),
        );
        method(
            &mut code,
            "encoding(&self, channel: Channel) -> Option<&Encoding>",
            &|variant, _, _| format!("Self::{variant}(mark) => mark.encoding(channel)"),
        );
        method(
            &mut code,
            "encoding_mut(&mut self, channel: Channel) -> Option<&mut Encoding>",
            &|variant, _, _| format!("Self::{variant}(mark) => mark.encoding_mut(channel)"),
        );
        code.push_str("}\n\n");
    }

    writeln!(code, "impl Channel {{").unwrap();
    writeln!(
        code,
        "    /// The name of the field of the channel in a mark, such as `stroke_dash`."
    )
    .unwrap();
    writeln!(
        code,
        "    pub fn field_name(self) -> &'static str {{\n        match self {{"
    )
    .unwrap();
    writeln!(code, "            Self::Unspecified => \"\",").unwrap();
    for (name, _) in &channels {
        writeln!(code, "            Self::{} => \"{name}\",", camel(name)).unwrap();
    }
    writeln!(code, "        }}\n    }}\n}}").unwrap();
    Ok(code)
}

/// The channels and the encodings of the mark at `path`.
fn write_mark(code: &mut String, path: &str, taken: &[(String, Holder)]) {
    let variants: Vec<String> = taken.iter().map(|(name, _)| camel(name)).collect();
    writeln!(code, "impl {path} {{").unwrap();
    writeln!(
        code,
        "    /// The channels that the mark takes, in the order of `Channel`."
    )
    .unwrap();
    writeln!(
        code,
        "    pub const CHANNELS: &'static [Channel] = &[{}];",
        (variants.iter())
            .map(|variant| format!("Channel::{variant}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
    .unwrap();
    for (signature, reference) in [
        (
            "encoding(&self, channel: Channel) -> Option<&Encoding>",
            "&",
        ),
        (
            "encoding_mut(&mut self, channel: Channel) -> Option<&mut Encoding>",
            "&mut ",
        ),
    ] {
        let doc = match reference {
            "&" => "The encoding of `channel`, or `None` when the mark does not encode it.",
            _ => {
                "The encoding of `channel`, to change, or `None` when the mark does not encode it."
            }
        };
        writeln!(
            code,
            "    /// {doc}\n    pub fn {signature} {{\n        match channel {{"
        )
        .unwrap();
        for ((_, holder), variant) in taken.iter().zip(&variants) {
            let arm = match holder {
                Holder::Field(field) => match reference {
                    "&" => format!("self.{field}.as_ref()"),
                    _ => format!("self.{field}.as_mut()"),
                },
                Holder::Oneof { field, path } => format!(
                    "match {reference}self.{field} {{ Some({path}::{variant}(encoding)) => Some(encoding), _ => None }}"
                ),
            };
            writeln!(code, "            Channel::{variant} => {arm},").unwrap();
        }
        writeln!(code, "            _ => None,\n        }}\n    }}").unwrap();
    }
    code.push_str("}\n\n");
}

/// The parts of the descriptors that hold the options of enum values. `prost_types` drops the
/// extensions of options when it decodes, so these messages decode the `invalid` option of
/// `annotation.proto` by its field number.
mod annotations {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct FileSet {
        #[prost(message, repeated, tag = "1")]
        pub file: Vec<File>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct File {
        #[prost(message, repeated, tag = "5")]
        pub enum_type: Vec<Enum>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Enum {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(message, repeated, tag = "2")]
        pub value: Vec<Value>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Value {
        #[prost(int32, optional, tag = "2")]
        pub number: Option<i32>,
        #[prost(message, optional, tag = "3")]
        pub options: Option<ValueOptions>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ValueOptions {
        #[prost(bool, optional, tag = "50000")]
        pub invalid: Option<bool>,
    }
}

/// The values that the `invalid` option marks, by the full name of their enum and their number,
/// from the bytes of the descriptor set.
fn invalid_values(bytes: &[u8]) -> Result<BTreeSet<(String, i32)>, prost::DecodeError> {
    let set = annotations::FileSet::decode(bytes)?;
    let mut invalid = BTreeSet::new();
    for file in &set.file {
        for enumeration in &file.enum_type {
            for value in &enumeration.value {
                if value.options.as_ref().and_then(|options| options.invalid) == Some(true) {
                    invalid.insert((
                        format!(
                            "{PACKAGE}.{}",
                            enumeration.name.as_deref().unwrap_or_default()
                        ),
                        value.number.unwrap_or_default(),
                    ));
                }
            }
        }
    }
    Ok(invalid)
}

/// The code of `enum_error`, which takes the full name of an enum and a value of a field of it,
/// and returns why the value is not valid, or `None`: a value that the enum does not declare, or
/// one that the `invalid` option marks.
fn enum_error(code: &mut String, set: &FileDescriptorSet, invalid: &BTreeSet<(String, i32)>) {
    let enums: Vec<(String, &EnumDescriptorProto)> = (set.file.iter())
        .flat_map(|file| &file.enum_type)
        .map(|enumeration| (format!("{PACKAGE}.{}", enumeration.name()), enumeration))
        .collect();
    code.push_str(
        "/// Why `value` is not a valid value of a field of the enum named `name`, or `None`.\n",
    );
    code.push_str("pub(crate) fn enum_error(name: &str, value: i32) -> Option<&'static str> {\n");
    code.push_str("    match name {\n");
    for (name, enumeration) in &enums {
        let is_invalid = |number: i32| invalid.contains(&(name.clone(), number));
        let mut declared: Vec<i32> = (enumeration.value.iter())
            .map(|value| value.number())
            .filter(|number| !is_invalid(*number))
            .collect();
        declared.sort_unstable();
        // The declared values as patterns, with each run of consecutive values as a range.
        let mut numbers: Vec<String> = Vec::new();
        for run in declared.chunk_by(|a, b| b - a == 1) {
            numbers.push(match run {
                [one] => one.to_string(),
                [first, .., last] => format!("{first}..={last}"),
                [] => unreachable!("a chunk is not empty"),
            });
        }
        let short = enumeration.name();
        writeln!(code, "        \"{name}\" => match value {{").unwrap();
        for value in (enumeration.value.iter()).filter(|value| is_invalid(value.number())) {
            let reason = match value.number() {
                0 => format!("is absent, and a value of `{short}` is required"),
                _ => format!("is `{}`, which is not a valid value", value.name()),
            };
            writeln!(
                code,
                "            {} => Some(\"{reason}\"),",
                value.number()
            )
            .unwrap();
        }
        writeln!(code, "            {} => None,", numbers.join(" | ")).unwrap();
        writeln!(
            code,
            "            _ => Some(\"is not a value of `{short}`\"),\n        }},"
        )
        .unwrap();
    }
    code.push_str("        _ => None,\n    }\n}\n\n");
}

/// The messages of the package, each with its full name and its Rust path.
struct Messages<'a> {
    all: Vec<(String, String, &'a DescriptorProto)>,
}

impl<'a> Messages<'a> {
    fn new(set: &'a FileDescriptorSet) -> Self {
        let mut all = Vec::new();
        for file in &set.file {
            for message in &file.message_type {
                Self::collect(&mut all, PACKAGE, "crate::proto", message);
            }
        }
        Self { all }
    }

    fn collect(
        all: &mut Vec<(String, String, &'a DescriptorProto)>,
        scope: &str,
        module: &str,
        message: &'a DescriptorProto,
    ) {
        let name = message.name();
        all.push((
            format!("{scope}.{name}"),
            format!("{module}::{name}"),
            message,
        ));
        for nested in &message.nested_type {
            Self::collect(
                all,
                &format!("{scope}.{name}"),
                &format!("{module}::{}", snake(name)),
                nested,
            );
        }
    }

    fn get(&self, name: &str) -> Option<&(String, String, &'a DescriptorProto)> {
        self.all.iter().find(|(full, _, _)| full == name)
    }

    /// The messages that hold a `target`, directly or through their fields.
    fn holders(&self, target: &str) -> BTreeSet<String> {
        let mut holders = BTreeSet::new();
        loop {
            let before = holders.len();
            for (name, _, message) in &self.all {
                if message
                    .field
                    .iter()
                    .any(|field| field.type_name() == target || holders.contains(field.type_name()))
                {
                    holders.insert(name.clone());
                }
            }
            if holders.len() == before {
                return holders;
            }
        }
    }

    /// The messages that the specs of a shared scale merge field by field: the guides, the
    /// messages of the scale types, and the fits, with the messages that they hold.
    fn merged(&self) -> BTreeSet<String> {
        let mut merged: BTreeSet<String> = ["Guide", "DomainFit"]
            .iter()
            .map(|name| format!("{PACKAGE}.{name}"))
            .collect();
        if let Some((_, _, spec)) = self.get(&format!("{PACKAGE}.ScaleSpec")) {
            for field in &spec.field {
                // `merge.rs` merges the options of custom scales by name, because they are keyed.
                if field.oneof_index.is_some()
                    && field.r#type() == Type::Message
                    && field.type_name() != format!("{PACKAGE}.CustomScale")
                {
                    merged.insert(field.type_name().to_string());
                }
            }
        }
        loop {
            let before = merged.len();
            for (name, _, message) in &self.all {
                if merged.contains(name) {
                    for field in &message.field {
                        if field.r#type() == Type::Message
                            && field.type_name() != format!("{PACKAGE}.ValueRef")
                        {
                            merged.insert(field.type_name().to_string());
                        }
                    }
                }
            }
            if merged.len() == before {
                return merged;
            }
        }
    }

    /// An impl of `MergeFields` for each message of `merged`. A field that one message sets and
    /// the other does not takes the set value, and a field that both set must be equal: an
    /// `optional` field is set when it is `Some`, a plain string when it is not empty, an enum when
    /// it is not its zero value, a message when it is `Some`, whose fields merge in turn, a oneof
    /// when it is `Some`, whose members must be of one kind, and a repeated field when it is not
    /// empty. A `ValueRef` is equal to another that refers to an equal value.
    fn merges(&self) -> String {
        let merged = self.merged();
        let value_ref = format!("{PACKAGE}.ValueRef");
        let mut code = String::new();
        for (name, path, message) in &self.all {
            if !merged.contains(name) {
                continue;
            }
            let module = path.rsplit_once("::").expect("a nested path").0.to_string()
                + "::"
                + &snake(message.name());
            let mut body = String::new();
            let mut oneofs_done = BTreeSet::new();
            for field in &message.field {
                let field_ident = ident(field.name());
                let at = format!("&format!(\"{{path}}.{}\")", field.name());
                match field.oneof_index {
                    Some(oneof) if !field.proto3_optional() => {
                        if !oneofs_done.insert(oneof) {
                            continue;
                        }
                        let oneof_name = message.oneof_decl[oneof as usize].name();
                        let oneof_ident = ident(oneof_name);
                        let members: Vec<_> = (message.field.iter())
                            .filter(|member| member.oneof_index == Some(oneof))
                            .collect();
                        writeln!(
                            body,
                            "        match (&mut self.{oneof_ident}, &other.{oneof_ident}) {{"
                        )
                        .unwrap();
                        body.push_str("            (_, None) => {}\n");
                        writeln!(
                            body,
                            "            (merged @ None, Some(other)) => *merged = Some(other.clone()),"
                        )
                        .unwrap();
                        for member in members {
                            let variant = format!(
                                "{module}::{}::{}",
                                camel(oneof_name),
                                camel(member.name())
                            );
                            let member_at = format!("&format!(\"{{path}}.{}\")", member.name());
                            let arm = if member.r#type() == Type::Message
                                && member.type_name() == value_ref
                            {
                                format!("if !same(*merged, *other) {{ return Err(differ({member_at})); }}")
                            } else if member.r#type() == Type::Message
                                && merged.contains(member.type_name())
                            {
                                format!("merged.merge_fields(other, same, {member_at})?")
                            } else {
                                format!("if merged != other {{ return Err(differ({member_at})); }}")
                            };
                            writeln!(
                                body,
                                "            (Some({variant}(merged)), Some({variant}(other))) => {{ {arm} }}"
                            )
                            .unwrap();
                        }
                        writeln!(
                            body,
                            "            _ => return Err(differ(&format!(\"{{path}}.{oneof_name}\"))),\n        }}"
                        )
                        .unwrap();
                    }
                    _ => {
                        let call = if field.label() == Label::Repeated {
                            "repeated".to_string()
                        } else if field.r#type() == Type::Message && field.type_name() == value_ref
                        {
                            "reference".to_string()
                        } else if field.r#type() == Type::Message
                            && merged.contains(field.type_name())
                        {
                            writeln!(
                                body,
                                "        match (&mut self.{field_ident}, &other.{field_ident}) {{\n            (_, None) => {{}}\n            (merged @ None, Some(other)) => *merged = Some(other.clone()),\n            (Some(merged), Some(other)) => merged.merge_fields(other, same, {at})?,\n        }}"
                            )
                            .unwrap();
                            continue;
                        } else if field.proto3_optional() || field.r#type() == Type::Message {
                            "optional".to_string()
                        } else if field.r#type() == Type::String {
                            "text".to_string()
                        } else if field.r#type() == Type::Enum {
                            "enumeration".to_string()
                        } else {
                            "plain".to_string()
                        };
                        let extra = if call == "reference" { ", same" } else { "" };
                        writeln!(
                            body,
                            "        {call}(&mut self.{field_ident}, &other.{field_ident}{extra}, {at})?;"
                        )
                        .unwrap();
                    }
                }
            }
            // prost derives `Copy` for some messages and oneofs, by their fields, so the clones of
            // those values are copies.
            code.push_str("#[allow(clippy::clone_on_copy)]\n");
            writeln!(code, "impl MergeFields for {path} {{").unwrap();
            writeln!(
                code,
                "    fn merge_fields(&mut self, other: &Self, same: &SameValue, path: &str) -> Merged {{"
            )
            .unwrap();
            if body.is_empty() {
                code.push_str("        let _ = (other, same, path);\n");
            } else if !body.contains("same") {
                code.push_str("        let _ = same;\n");
            }
            code.push_str(&body);
            code.push_str("        Ok(())\n    }\n}\n\n");
        }
        code
    }

    /// The messages that hold an enum field, directly or through their fields.
    fn enum_holders(&self) -> BTreeSet<String> {
        let mut holders = BTreeSet::new();
        loop {
            let before = holders.len();
            for (name, _, message) in &self.all {
                if (message.field.iter()).any(|field| {
                    field.r#type() == Type::Enum || holders.contains(field.type_name())
                }) {
                    holders.insert(name.clone());
                }
            }
            if holders.len() == before {
                return holders;
            }
        }
    }

    /// An impl of `VisitEnums` for each message that holds an enum field. Its method calls its
    /// function with the path of each enum field from the message, the full name of the field's
    /// enum, and the field's value.
    fn enum_visitors(&self, code: &mut String) {
        let holders = self.enum_holders();
        for (name, path, message) in &self.all {
            if !holders.contains(name) {
                continue;
            }
            let module = path.rsplit_once("::").expect("a nested path").0.to_string()
                + "::"
                + &snake(message.name());
            // The call for `item` at the path `at`: the function for an enum, and the item's own
            // walk for a message.
            let call = |field: &FieldDescriptorProto, item: &str, at: &str| match field.r#type() {
                Type::Enum => format!("f(&{at}, \"{}\", *{item})", field.type_name()),
                _ => format!("{item}.visit_enums(&{at}, f)"),
            };
            let mut body = String::new();
            let mut oneofs_done = BTreeSet::new();
            for field in &message.field {
                let relevant = |field: &FieldDescriptorProto| {
                    field.r#type() == Type::Enum || holders.contains(field.type_name())
                };
                let field_ident = ident(field.name());
                let at = format!("format!(\"{{path}}.{}\")", field.name());
                match field.oneof_index {
                    Some(oneof) if !field.proto3_optional() => {
                        if !oneofs_done.insert(oneof) {
                            continue;
                        }
                        let members: Vec<_> = (message.field.iter())
                            .filter(|member| member.oneof_index == Some(oneof) && relevant(member))
                            .collect();
                        if members.is_empty() {
                            continue;
                        }
                        let oneof_name = message.oneof_decl[oneof as usize].name();
                        let arms: Vec<(String, String)> = (members.iter())
                            .map(|member| {
                                let at = format!("format!(\"{{path}}.{}\")", member.name());
                                let pattern = format!(
                                    "Some({module}::{}::{}(item))",
                                    camel(oneof_name),
                                    camel(member.name())
                                );
                                (pattern, call(member, "item", &at))
                            })
                            .collect();
                        let oneof_field = ident(oneof_name);
                        match arms.as_slice() {
                            [(pattern, item)] => writeln!(
                                body,
                                "        if let {pattern} = &self.{oneof_field} {{ {item}; }}"
                            )
                            .unwrap(),
                            _ => {
                                writeln!(body, "        match &self.{oneof_field} {{").unwrap();
                                for (pattern, item) in &arms {
                                    writeln!(body, "            {pattern} => {item},").unwrap();
                                }
                                body.push_str("            _ => {}\n        }\n");
                            }
                        }
                    }
                    _ if !relevant(field) => {}
                    _ if field.label() == Label::Repeated => {
                        let at = format!("format!(\"{{path}}.{}[{{index}}]\")", field.name());
                        writeln!(
                            body,
                            "        for (index, item) in self.{field_ident}.iter().enumerate() {{ {}; }}",
                            call(field, "item", &at)
                        )
                        .unwrap();
                    }
                    _ if field.r#type() == Type::Enum && !field.proto3_optional() => {
                        writeln!(
                            body,
                            "        f(&{at}, \"{}\", self.{field_ident});",
                            field.type_name(),
                        )
                        .unwrap();
                    }
                    _ => {
                        writeln!(
                            body,
                            "        if let Some(item) = &self.{field_ident} {{ {}; }}",
                            call(field, "item", &at)
                        )
                        .unwrap();
                    }
                }
            }
            writeln!(code, "impl VisitEnums for {path} {{").unwrap();
            writeln!(
                code,
                "    fn visit_enums(&self, path: &str, f: &mut dyn FnMut(&str, &'static str, i32)) {{"
            )
            .unwrap();
            code.push_str(&body);
            code.push_str("    }\n}\n\n");
        }
    }

    /// An impl of `visitor` for each message that holds a `target`, with a method `{method}` that
    /// reads each `target` and a method `{method}_mut` that changes it. Each calls its function
    /// with each `target` in the order that the `.proto` files declare the fields, where a oneof
    /// takes the place of its first member.
    fn visitors(&self, code: &mut String, target: &str, visitor: &str, method: &str) {
        let holders = self.holders(target);
        let (_, target_path, _) = self.get(target).expect("the target is a message");
        for (name, path, message) in &self.all {
            if !holders.contains(name) {
                continue;
            }
            writeln!(code, "impl {visitor} for {path} {{").unwrap();
            for (suffix, reference) in [("", "&"), ("_mut", "&mut ")] {
                let visit = |item: &str, type_name: &str| match type_name == target {
                    true => format!("f({item})"),
                    false => format!("{item}.{method}{suffix}(f)"),
                };
                let module = path.rsplit_once("::").expect("a nested path").0.to_string()
                    + "::"
                    + &snake(message.name());
                let mut body = String::new();
                let mut oneofs_done = BTreeSet::new();
                for field in &message.field {
                    let relevant =
                        |type_name: &str| type_name == target || holders.contains(type_name);
                    let field_ident = ident(field.name());
                    match field.oneof_index {
                        Some(oneof) if !field.proto3_optional() => {
                            if !oneofs_done.insert(oneof) {
                                continue;
                            }
                            let members: Vec<_> = (message.field.iter())
                                .filter(|member| {
                                    member.oneof_index == Some(oneof)
                                        && relevant(member.type_name())
                                })
                                .collect();
                            if members.is_empty() {
                                continue;
                            }
                            let oneof_name = message.oneof_decl[oneof as usize].name();
                            let oneof_field = ident(oneof_name);
                            let arms: Vec<_> = (members.iter())
                                .map(|member| {
                                    let pattern = format!(
                                        "Some({module}::{}::{}(item))",
                                        camel(oneof_name),
                                        camel(member.name())
                                    );
                                    (pattern, visit("item", member.type_name()))
                                })
                                .collect();
                            match arms.as_slice() {
                                [(pattern, item)] => writeln!(
                                    body,
                                    "        if let {pattern} = {reference}self.{oneof_field} {{ {item}; }}"
                                )
                                .unwrap(),
                                _ => {
                                    writeln!(body, "        match {reference}self.{oneof_field} {{")
                                        .unwrap();
                                    for (pattern, item) in &arms {
                                        writeln!(body, "            {pattern} => {item},").unwrap();
                                    }
                                    body.push_str("            _ => {}\n        }\n");
                                }
                            }
                        }
                        _ if !relevant(field.type_name()) => {}
                        _ if field.label() == Label::Repeated => {
                            let item = visit("item", field.type_name());
                            writeln!(
                                body,
                                "        for item in {reference}self.{field_ident} {{ {item}; }}"
                            )
                            .unwrap();
                        }
                        _ => {
                            let item = visit("item", field.type_name());
                            writeln!(
                                body,
                                "        if let Some(item) = {reference}self.{field_ident} {{ {item}; }}"
                            )
                            .unwrap();
                        }
                    }
                }
                writeln!(
                    code,
                    "    fn {method}{suffix}(&{} self, f: &mut dyn FnMut({reference}{target_path})) {{",
                    if suffix.is_empty() { "" } else { "mut" }
                )
                .unwrap();
                code.push_str(&body);
                code.push_str("    }\n");
            }
            code.push_str("}\n\n");
        }
    }
}

/// The Rust identifier of a field or a oneof, as prost names it.
fn ident(name: &str) -> String {
    let name = snake(name);
    match name.as_str() {
        "type" | "ref" | "mod" | "match" | "in" | "fn" | "impl" | "use" | "where" | "move" => {
            format!("r#{name}")
        }
        _ => name,
    }
}

/// `name` in snake case, as prost names modules and fields.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `name` in upper camel case, as prost names oneof enums and their variants.
fn camel(name: &str) -> String {
    name.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}
