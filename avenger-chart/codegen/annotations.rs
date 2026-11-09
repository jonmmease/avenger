//! The options of `annotation.proto` on the fields, the oneofs, and the enum values of the package,
//! and the code that they generate: the presence checks of `VisitRequired` in `walk.rs`, and
//! `takes_scale` in `channels.rs`.
//!
//! prost drops the extensions of the descriptor options, so the options are read from the bytes
//! of the descriptor set with messages that declare only the fields and the extensions they need.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

use prost::Message;
use prost_types::{
    field_descriptor_proto::{Label, Type},
    FieldDescriptorProto, FileDescriptorSet,
};

use super::{camel, ident, snake, Messages, PACKAGE};

mod wire {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct FileSet {
        #[prost(message, repeated, tag = "1")]
        pub file: Vec<File>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct File {
        #[prost(string, optional, tag = "2")]
        pub package: Option<String>,
        #[prost(message, repeated, tag = "4")]
        pub message_type: Vec<Descriptor>,
        #[prost(message, repeated, tag = "5")]
        pub enum_type: Vec<Enum>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Descriptor {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(message, repeated, tag = "2")]
        pub field: Vec<Field>,
        #[prost(message, repeated, tag = "3")]
        pub nested_type: Vec<Descriptor>,
        #[prost(message, optional, tag = "7")]
        pub options: Option<MessageOptions>,
        #[prost(message, repeated, tag = "8")]
        pub oneof_decl: Vec<Oneof>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct MessageOptions {
        #[prost(string, optional, tag = "50006")]
        pub constructor: Option<String>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Field {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(message, optional, tag = "8")]
        pub options: Option<FieldOptions>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct FieldOptions {
        #[prost(bool, optional, tag = "50001")]
        pub required: Option<bool>,
        #[prost(bool, optional, tag = "50003")]
        pub lift: Option<bool>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Oneof {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(message, optional, tag = "2")]
        pub options: Option<OneofOptions>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct OneofOptions {
        #[prost(bool, optional, tag = "50002")]
        pub required_oneof: Option<bool>,
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
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(message, optional, tag = "3")]
        pub options: Option<ValueOptions>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ValueOptions {
        #[prost(int32, optional, tag = "50004")]
        pub channel_scale: Option<i32>,
        #[prost(bool, optional, tag = "50005")]
        pub position: Option<bool>,
    }
}

/// The value of `ChannelScale` for a channel with a scale of its own.
const CHANNEL_SCALE_OWN: i32 = 1;

/// The options of the package, by the full name of each message and the name of its field or
/// oneof.
#[derive(Default)]
pub struct Annotations {
    pub required: BTreeSet<(String, String)>,
    pub required_oneof: BTreeSet<(String, String)>,
    pub lift: BTreeSet<(String, String)>,
    /// The `constructor` of each message that sets it, by the message's full name.
    pub constructor: BTreeMap<String, String>,
    /// The `channel_scale` of each value of `Channel` that sets it, by the value's name.
    pub channel_scale: BTreeMap<String, i32>,
    /// The values of `Channel` that the `position` option marks, by name.
    pub positions: BTreeSet<String>,
}

impl Annotations {
    /// The options of the package in the descriptor set `bytes`.
    pub fn read(bytes: &[u8]) -> Result<Self, prost::DecodeError> {
        let set = wire::FileSet::decode(bytes)?;
        let mut annotations = Self::default();
        for file in &set.file {
            if format!(".{}", file.package()) != PACKAGE {
                continue;
            }
            for message in &file.message_type {
                annotations.message(PACKAGE, message);
            }
            for enumeration in (file.enum_type.iter()).filter(|e| e.name() == "Channel") {
                for value in &enumeration.value {
                    if let Some(scale) = value.options.as_ref().and_then(|o| o.channel_scale) {
                        annotations
                            .channel_scale
                            .insert(value.name().to_string(), scale);
                    }
                    if value.options.as_ref().and_then(|o| o.position) == Some(true) {
                        annotations.positions.insert(value.name().to_string());
                    }
                }
            }
        }
        Ok(annotations)
    }

    fn message(&mut self, scope: &str, message: &wire::Descriptor) {
        let name = format!("{scope}.{}", message.name());
        if let Some(constructor) = message.options.as_ref().and_then(|o| o.constructor.clone()) {
            self.constructor.insert(name.clone(), constructor);
        }
        for field in &message.field {
            let options = field.options.as_ref();
            let key = (name.clone(), field.name().to_string());
            if options.and_then(|o| o.required) == Some(true) {
                self.required.insert(key.clone());
            }
            if options.and_then(|o| o.lift) == Some(true) {
                self.lift.insert(key);
            }
        }
        for oneof in &message.oneof_decl {
            if oneof.options.as_ref().and_then(|o| o.required_oneof) == Some(true) {
                self.required_oneof
                    .insert((name.clone(), oneof.name().to_string()));
            }
        }
        for nested in &message.nested_type {
            self.message(&name, nested);
        }
    }

    pub fn is_required(&self, message: &str, field: &str) -> bool {
        self.required
            .contains(&(message.to_string(), field.to_string()))
    }

    pub fn is_required_oneof(&self, message: &str, oneof: &str) -> bool {
        self.required_oneof
            .contains(&(message.to_string(), oneof.to_string()))
    }
}

/// The name of the zero value of the enum named `name`, the full name of an enum of the package.
fn zero_value<'a>(set: &'a FileDescriptorSet, name: &str) -> &'a str {
    (set.file.iter())
        .flat_map(|file| &file.enum_type)
        .find(|enumeration| format!("{PACKAGE}.{}", enumeration.name()) == name)
        .and_then(|enumeration| enumeration.value.iter().find(|value| value.number() == 0))
        .map_or("0", |value| value.name())
}

/// The test that a required field `field`, at the Rust expression `value`, is absent, and the
/// reason that a check reports. A `ValueRef` or a message is present, a string is not empty, a
/// repeated field has an item, and an enum holds a value other than zero.
fn absent(
    set: &FileDescriptorSet,
    field: &FieldDescriptorProto,
    value: &str,
    member: bool,
) -> Result<(String, String), String> {
    let name = field.name();
    if field.label() == Label::Repeated {
        return Ok((
            format!("{value}.is_empty()"),
            format!("`{name}` holds one item or more"),
        ));
    }
    match field.r#type() {
        Type::Message if member => Err(format!(
            "the member `{name}` of a oneof is a message, which is present when the oneof holds it"
        )),
        Type::Message => Ok((
            format!("{value}.is_none()"),
            format!("`{name}` is required"),
        )),
        Type::String if !field.proto3_optional() => Ok((
            format!("{value}.is_empty()"),
            format!("`{name}` is required and not empty"),
        )),
        Type::Enum if !field.proto3_optional() => Ok((
            format!("{value} == 0"),
            format!(
                "`{name}` is required and not `{}`",
                zero_value(set, field.type_name())
            ),
        )),
        _ => Err(format!(
            "`{name}` is required, which only a message, a string, a repeated field, or an enum that \
             is not `optional` can be"
        )),
    }
}

/// The messages that hold a required field or a required oneof, directly or through their fields.
fn holders(messages: &Messages, annotations: &Annotations) -> BTreeSet<String> {
    let mut holders: BTreeSet<String> = (annotations.required.iter())
        .chain(&annotations.required_oneof)
        .map(|(message, _)| message.clone())
        .collect();
    loop {
        let before = holders.len();
        for (name, _, message) in &messages.all {
            if (message.field.iter()).any(|field| holders.contains(field.type_name())) {
                holders.insert(name.clone());
            }
        }
        if holders.len() == before {
            return holders;
        }
    }
}

/// An impl of `VisitRequired` for each message that holds a required field or a required oneof.
/// Its method calls its function with the path and the reason of each absent one, in the order of
/// the fields, depth first.
pub fn required_visitors(
    code: &mut String,
    set: &FileDescriptorSet,
    messages: &Messages,
    annotations: &Annotations,
) -> Result<(), String> {
    let holders = holders(messages, annotations);
    for (name, path, message) in &messages.all {
        if !holders.contains(name) {
            continue;
        }
        let module = path.rsplit_once("::").expect("a nested path").0.to_string()
            + "::"
            + &snake(message.name());
        let at = |field: &str| format!("field_path(path, \"{field}\")");
        let mut body = String::new();
        let mut oneofs_done = BTreeSet::new();
        for field in &message.field {
            let field_ident = ident(field.name());
            let nested = holders.contains(field.type_name());
            match field.oneof_index {
                Some(oneof) if !field.proto3_optional() => {
                    if !oneofs_done.insert(oneof) {
                        continue;
                    }
                    let oneof_name = message.oneof_decl[oneof as usize].name();
                    let members: Vec<&FieldDescriptorProto> = (message.field.iter())
                        .filter(|member| {
                            member.oneof_index == Some(oneof) && !member.proto3_optional()
                        })
                        .collect();
                    let oneof_field = ident(oneof_name);
                    if annotations.is_required_oneof(name, oneof_name) {
                        // The reason lists the members of a short oneof.
                        let reason = match members.len() {
                            0..=6 => {
                                let names: Vec<String> = (members.iter())
                                    .map(|m| format!("`{}`", m.name()))
                                    .collect();
                                format!("`{oneof_name}` is required: one of {}", names.join(", "))
                            }
                            _ => format!("`{oneof_name}` is required"),
                        };
                        writeln!(
                            body,
                            "        if self.{oneof_field}.is_none() {{ f(path.to_string(), \"{reason}\"); }}"
                        )
                        .unwrap();
                    }
                    let mut arms: Vec<(String, Option<String>, String)> = Vec::new();
                    for member in &members {
                        let pattern = format!(
                            "Some({module}::{}::{}(item))",
                            camel(oneof_name),
                            camel(member.name())
                        );
                        if annotations.is_required(name, member.name()) {
                            let (test, reason) = absent(set, member, "(*item)", true)
                                .map_err(|e| format!("`{}`: {e}", message.name()))?;
                            arms.push((
                                pattern.clone(),
                                Some(test),
                                format!("f({}, \"{reason}\")", at(member.name())),
                            ));
                        }
                        if holders.contains(member.type_name()) {
                            arms.push((
                                pattern,
                                None,
                                format!("item.visit_required(&{}, f)", at(member.name())),
                            ));
                        }
                    }
                    match arms.as_slice() {
                        [] => {}
                        [(pattern, None, call)] => writeln!(
                            body,
                            "        if let {pattern} = &self.{oneof_field} {{ {call}; }}"
                        )
                        .unwrap(),
                        _ => {
                            writeln!(body, "        match &self.{oneof_field} {{").unwrap();
                            for (pattern, guard, call) in &arms {
                                let guard =
                                    guard.as_ref().map_or(String::new(), |g| format!(" if {g}"));
                                writeln!(body, "            {pattern}{guard} => {call},").unwrap();
                            }
                            body.push_str("            _ => {}\n        }\n");
                        }
                    }
                }
                _ => {
                    if annotations.is_required(name, field.name()) {
                        let (test, reason) =
                            absent(set, field, &format!("self.{field_ident}"), false)
                                .map_err(|e| format!("`{}`: {e}", message.name()))?;
                        writeln!(
                            body,
                            "        if {test} {{ f({}, \"{reason}\"); }}",
                            at(field.name())
                        )
                        .unwrap();
                    }
                    if !nested {
                        continue;
                    }
                    if field.label() == Label::Repeated {
                        writeln!(
                            body,
                            "        for (index, item) in self.{field_ident}.iter().enumerate() {{ item.visit_required(&format!(\"{{}}[{{index}}]\", {}), f); }}",
                            at(field.name())
                        )
                        .unwrap();
                    } else {
                        writeln!(
                            body,
                            "        if let Some(item) = &self.{field_ident} {{ item.visit_required(&{}, f); }}",
                            at(field.name())
                        )
                        .unwrap();
                    }
                }
            }
        }
        writeln!(code, "impl VisitRequired for {path} {{").unwrap();
        writeln!(
            code,
            "    fn visit_required(&self, path: &str, f: &mut dyn FnMut(String, &'static str)) {{"
        )
        .unwrap();
        code.push_str(&body);
        code.push_str("    }\n}\n\n");
    }
    Ok(())
}

/// The code of `channels.rs`, from the `channel_scale` and `position` options of the values of
/// `Channel`: `takes_scale`, `is_position`, and what a string and a number mean in the builder of
/// an encoding of a channel.
pub fn channels(annotations: &Annotations) -> String {
    let variant = |name: &str| {
        format!(
            "Channel::{}",
            camel(&name.strip_prefix("CHANNEL_").unwrap_or(name).to_lowercase())
        )
    };
    let matching = |names: Vec<&String>| -> String {
        names
            .into_iter()
            .map(|name| variant(name))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let own = matching(
        (annotations.channel_scale.iter())
            .filter(|(_, scale)| **scale == CHANNEL_SCALE_OWN)
            .map(|(name, _)| name)
            .collect(),
    );
    let scaled = matching(annotations.channel_scale.keys().collect());
    let positions = matching(annotations.positions.iter().collect());
    let scaled_positions = matching(
        (annotations.positions.iter())
            .filter(|name| annotations.channel_scale.contains_key(*name))
            .collect(),
    );
    format!(
        "/// Whether a channel has a scale of its own, as the `channel_scale` option of its value \
         says: not the second\n/// channel of a pair, which follows the scale of the first, and \
         not a channel of direct values.\npub(crate) fn takes_scale(channel: Channel) -> bool {{\n    \
         matches!(channel, {own})\n}}\n\n\
         /// Whether a channel is a position of a coordinate system, as the `position` option of \
         its value says.\npub(crate) fn is_position(channel: Channel) -> bool {{\n    \
         matches!(channel, {positions})\n}}\n\n\
         /// Whether a string on a channel names a scaled field: on a channel with a scale of its \
         own or of its partner.\npub(crate) fn scales_strings(channel: Channel) -> bool {{\n    \
         matches!(channel, {scaled})\n}}\n\n\
         /// Whether a number on a channel is a scaled value: on a position with a scale.\n\
         pub(crate) fn scales_numbers(channel: Channel) -> bool {{\n    \
         matches!(channel, {scaled_positions})\n}}\n"
    )
}
