//! Generates the prost messages of package `avenger.chart.v1` from the files in
//! `proto/avenger/chart/v1/` with the vendored `protoc`, so that a build needs no system `protoc`.
//!
//! It also reads the descriptors of the messages and generates two files:
//!
//! - `values.rs`: an impl of `VisitValues` for each message that holds a `ValueRef`, and of
//!   `VisitTables` for each message that holds a `TableRef`, directly or through its fields.
//! - `marks.rs`: for each member of the `Mark` oneof, its channels and their encodings, and for
//!   `Mark`, its kind, its type, its name, and the channels of its member.
//!
//! It fails when a field of a mark breaks a law of the format: an `Encoding` field has the name of
//! a `Channel` value, lowercase and without the prefix, and the number 100 plus that value, and
//! every other field, a property, has a number below 100.

use std::{collections::BTreeSet, fmt::Write, path::PathBuf};

use prost::Message;
use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, EnumDescriptorProto, FileDescriptorSet,
};

const PROTOS: [&str; 23] = [
    "attribution",
    "axis",
    "cartesian",
    "channel",
    "chart",
    "coord",
    "encoding",
    "facet",
    "geo",
    "guide",
    "interaction",
    "legend",
    "mark",
    "option",
    "partition",
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
    let mut values = String::new();
    messages.visitors(
        &mut values,
        &format!("{PACKAGE}.ValueRef"),
        "VisitValues",
        "visit_values_mut",
    );
    messages.visitors(
        &mut values,
        &format!("{PACKAGE}.TableRef"),
        "VisitTables",
        "visit_tables_mut",
    );
    std::fs::write(out.join("values.rs"), values)?;
    std::fs::write(out.join("marks.rs"), marks(&set, &messages)?)?;
    Ok(())
}

/// The kinds of coordinate systems, by the prefix of the names of their marks.
const KINDS: [(&str, &str); 3] = [("polar_", "Polar"), ("geo_", "Geo"), ("", "Cartesian")];

/// A member of the `Mark` oneof: its variant, its name, the path of its message, its kind, and
/// its type.
struct Member {
    variant: String,
    name: String,
    path: String,
    kind: &'static str,
    family: String,
}

/// The code of `marks.rs`, after a check of the laws of the fields of each mark.
fn marks(set: &FileDescriptorSet, messages: &Messages) -> Result<String, String> {
    let channel_enum: &EnumDescriptorProto = set
        .file
        .iter()
        .flat_map(|file| &file.enum_type)
        .find(|enumeration| enumeration.name() == "Channel")
        .ok_or("the package has a `Channel` enum")?;
    let channels: Vec<(String, i32)> = channel_enum
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
    let (_, _, mark) = messages
        .get(&format!("{PACKAGE}.Mark"))
        .ok_or("the package has a `Mark` message")?;
    let members: Vec<_> = mark.field.iter().collect();
    let cartesian: Vec<&str> = members
        .iter()
        .map(|member| member.name())
        .filter(|name| !name.starts_with("polar_") && !name.starts_with("geo_"))
        .collect();

    let mut code = String::new();
    let mut dispatch = Vec::new();
    for member in &members {
        let (_, path, message) = messages
            .get(member.type_name())
            .ok_or_else(|| format!("`Mark.{}` is a message", member.name()))?;
        let &(prefix, kind) = KINDS
            .iter()
            .find(|(prefix, _)| member.name().starts_with(prefix))
            .expect("the last kind has no prefix");
        let rest = &member.name()[prefix.len()..];
        // A member whose name without its prefix names a Cartesian mark is of that mark's type,
        // and any other member is a type of its own.
        let family = match cartesian.contains(&rest) {
            true => rest,
            false => member.name(),
        };
        let mut taken = Vec::new();
        for field in &message.field {
            let at = format!("`{}.{}`", message.name(), field.name());
            if field.r#type() == Type::Message && field.type_name() == format!("{PACKAGE}.Encoding")
            {
                let (name, value) = channels
                    .iter()
                    .find(|(name, _)| name == field.name())
                    .ok_or_else(|| format!("the encoding field {at} has the name of a channel"))?;
                if field.number() != 100 + value {
                    return Err(format!(
                        "the encoding field {at} has the number {}, 100 plus its channel's value",
                        100 + value
                    ));
                }
                taken.push(name.clone());
            } else if field.number() >= 100 {
                return Err(format!("the property {at} has a number below 100"));
            }
        }
        let variants: Vec<String> = taken.iter().map(|name| camel(name)).collect();
        writeln!(code, "impl {path} {{").unwrap();
        writeln!(
            code,
            "    /// The channels that the mark takes, in the order of `Channel`."
        )
        .unwrap();
        writeln!(
            code,
            "    pub const CHANNELS: &'static [Channel] = &[{}];",
            variants
                .iter()
                .map(|variant| format!("Channel::{variant}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        writeln!(
            code,
            "    /// The encoding of `channel`, or `None` when the mark does not encode it."
        )
        .unwrap();
        writeln!(
            code,
            "    pub fn encoding(&self, channel: Channel) -> Option<&Encoding> {{"
        )
        .unwrap();
        writeln!(code, "        match channel {{").unwrap();
        for (name, variant) in taken.iter().zip(&variants) {
            writeln!(
                code,
                "            Channel::{variant} => self.{}.as_ref(),",
                ident(name)
            )
            .unwrap();
        }
        writeln!(code, "            _ => None,\n        }}\n    }}").unwrap();
        writeln!(
            code,
            "    /// The field of `channel`, or `None` when the mark does not take it."
        )
        .unwrap();
        code.push_str("    pub fn encoding_mut(&mut self, channel: Channel)");
        code.push_str(" -> Option<&mut Option<Encoding>> {\n        match channel {\n");
        for (name, variant) in taken.iter().zip(&variants) {
            writeln!(
                code,
                "            Channel::{variant} => Some(&mut self.{}),",
                ident(name)
            )
            .unwrap();
        }
        writeln!(code, "            _ => None,\n        }}\n    }}\n}}\n").unwrap();
        let variant = camel(member.name());
        writeln!(code, "impl From<{path}> for Mark {{").unwrap();
        writeln!(code, "    fn from(mark: {path}) -> Self {{").unwrap();
        writeln!(code, "        Self::{variant}(mark)\n    }}\n}}\n").unwrap();
        dispatch.push(Member {
            variant: camel(member.name()),
            name: member.name().to_string(),
            path: path.clone(),
            kind,
            family: camel(family),
        });
    }

    // One match over the members of `Mark` for each method, with an arm from `arm`.
    let method =
        |code: &mut String, doc: &str, signature: &str, arm: &dyn Fn(&Member) -> String| {
            writeln!(
                code,
                "    /// {doc}\n    pub fn {signature} {{\n        match self {{"
            )
            .unwrap();
            for member in &dispatch {
                writeln!(code, "            {},", arm(member)).unwrap();
            }
            code.push_str("        }\n    }\n");
        };
    code.push_str("impl Mark {\n");
    method(
        &mut code,
        "The kind of the coordinate system that the mark draws in.",
        "kind(&self) -> CoordKind",
        &|m| format!("Self::{}(_) => CoordKind::{}", m.variant, m.kind),
    );
    method(
        &mut code,
        "The type of the mark, which the marks of each kind share.",
        "mark_type(&self) -> MarkType",
        &|m| format!("Self::{}(_) => MarkType::{}", m.variant, m.family),
    );
    method(
        &mut code,
        "The name of the mark's member of the `Mark` oneof, such as `polar_symbol`.",
        "name(&self) -> &'static str",
        &|m| format!("Self::{}(_) => \"{}\"", m.variant, m.name),
    );
    method(
        &mut code,
        "The channels that the mark takes, in the order of `Channel`.",
        "channels(&self) -> &'static [Channel]",
        &|m| format!("Self::{}(_) => {}::CHANNELS", m.variant, m.path),
    );
    method(
        &mut code,
        "The encoding of `channel`, or `None` when the mark does not encode it.",
        "encoding(&self, channel: Channel) -> Option<&Encoding>",
        &|m| format!("Self::{}(mark) => mark.encoding(channel)", m.variant),
    );
    method(
        &mut code,
        "The field of `channel`, or `None` when the mark does not take it.",
        "encoding_mut(&mut self, channel: Channel) -> Option<&mut Option<Encoding>>",
        &|m| format!("Self::{}(mark) => mark.encoding_mut(channel)", m.variant),
    );
    code.push_str("}\n\n");

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

    /// An impl of `visitor` for each message that holds a `target`. The method calls its function
    /// with each `target` in the order that the `.proto` files declare the fields, where a oneof
    /// takes the place of its first member.
    fn visitors(&self, code: &mut String, target: &str, visitor: &str, method: &str) {
        let holders = self.holders(target);
        let (_, target_path, _) = self.get(target).expect("the target is a message");
        let visit = |item: &str, type_name: &str| match type_name == target {
            true => format!("f({item})"),
            false => format!("{item}.{method}(f)"),
        };
        for (name, path, message) in &self.all {
            if !holders.contains(name) {
                continue;
            }
            let module = path.rsplit_once("::").expect("a nested path").0.to_string()
                + "::"
                + &snake(message.name());
            let mut body = String::new();
            let mut oneofs_done = BTreeSet::new();
            for field in &message.field {
                let relevant = |type_name: &str| type_name == target || holders.contains(type_name);
                let field_ident = ident(field.name());
                match field.oneof_index {
                    Some(oneof) if !field.proto3_optional() => {
                        if !oneofs_done.insert(oneof) {
                            continue;
                        }
                        let members: Vec<_> = message
                            .field
                            .iter()
                            .filter(|member| {
                                member.oneof_index == Some(oneof) && relevant(member.type_name())
                            })
                            .collect();
                        if members.is_empty() {
                            continue;
                        }
                        let oneof_name = message.oneof_decl[oneof as usize].name();
                        let oneof_field = ident(oneof_name);
                        let arms: Vec<_> = members
                            .iter()
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
                                "        if let {pattern} = &mut self.{oneof_field} {{ {item}; }}"
                            )
                            .unwrap(),
                            _ => {
                                writeln!(body, "        match &mut self.{oneof_field} {{").unwrap();
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
                            "        for item in &mut self.{field_ident} {{ {item}; }}"
                        )
                        .unwrap();
                    }
                    _ => {
                        let item = visit("item", field.type_name());
                        writeln!(
                            body,
                            "        if let Some(item) = &mut self.{field_ident} {{ {item}; }}"
                        )
                        .unwrap();
                    }
                }
            }
            writeln!(code, "impl {visitor} for {path} {{").unwrap();
            writeln!(
                code,
                "    fn {method}(&mut self, f: &mut dyn FnMut(&mut {target_path})) {{"
            )
            .unwrap();
            code.push_str(&body);
            code.push_str("    }\n}\n\n");
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
