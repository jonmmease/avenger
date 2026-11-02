//! Generates the prost messages of package `avenger.chart.v1` from the files in
//! `proto/avenger/chart/v1/` with the vendored `protoc`, so that a build needs no system `protoc`.
//!
//! It also reads the descriptors of the messages and generates `values.rs`: an impl of
//! `VisitValues` for each message that holds a `ValueRef`, and of `VisitTables` for each message
//! that holds a `TableRef`, directly or through its fields.

use std::{collections::BTreeSet, fmt::Write, path::PathBuf};

use prost::Message;
use prost_types::{field_descriptor_proto::Label, DescriptorProto, FileDescriptorSet};

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
    Ok(())
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
