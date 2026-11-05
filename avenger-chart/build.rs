//! Generates the prost messages of package `avenger.chart.v1` from the files in
//! `proto/avenger/chart/v1/` with the vendored `protoc`, so that a build needs no system `protoc`.
//!
//! It also reads the descriptors of the messages and generates two files:
//!
//! - `walk.rs`: impls of `VisitValues` for each message that holds a `ValueRef`, and of
//!   `VisitTables` for each message that holds a `TableRef`, directly or through its fields.
//! - `marks.rs`: for each mark, the member of the `mark` oneof of a layer, its channels and their
//!   encodings, and for each `mark` oneof, an impl of `MarkMessage`.
//!
//! It fails when a field of a mark breaks a law of the format: an `Encoding` field has the name of
//! a `Channel` value, lowercase and without the prefix, and the number 100 plus that value, and
//! every other field, a property, has a number below 100.

use std::{collections::BTreeSet, fmt::Write, path::PathBuf};

use prost::Message;
use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, FieldDescriptorProto, FileDescriptorSet,
};

const PROTOS: [&str; 24] = [
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
        .compile_protos(&files, &["proto"])?;

    let set = FileDescriptorSet::decode(std::fs::read(&descriptors)?.as_slice())?;
    let messages = Messages::new(&set);
    let mut walk = String::new();
    for (target, visitor, method) in [
        ("ValueRef", "VisitValues", "visit_values"),
        ("TableRef", "VisitTables", "visit_tables"),
    ] {
        messages.visitors(&mut walk, &format!("{PACKAGE}.{target}"), visitor, method);
    }
    std::fs::write(out.join("walk.rs"), walk)?;
    std::fs::write(out.join("marks.rs"), marks(&set, &messages)?)?;
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
