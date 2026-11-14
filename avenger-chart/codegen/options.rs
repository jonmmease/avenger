//! The option tree: the messages that the options of a chart reach from SQL, and the laws that
//! their oneofs keep, so the argument of a oneof's parameter picks one member.
//!
//! The option tree holds the messages that these fields reach through message fields:
//!
//! - the options of `Encoding`, its fields other than the members of `kind` and `conditional`
//! - the mark messages, the members of the `mark` oneof of each layer, and the fields of each that
//!   are not channels
//! - the `options` of each plot of a coordinate system: `CartesianPlot`, `PolarPlot`, and `GeoPlot`
//! - the composition nodes `Plot`, `Concat`, and `Wrap`, whose fields other than the structural ones
//!   are the options of the compositions that build them
//!
//! The tree stops at `ValueRef`, `TableRef`, `Table`, and `Encoding`, which are values, tables,
//! and channels, not options. build.rs fails, in every build, when a oneof of the tree has two
//! members that its argument cannot tell apart, when a member without fields, which its name
//! picks, shares its oneof with a member that takes a string, or when a message field outside a
//! oneof has a message without fields, which no argument sets.

use std::collections::BTreeSet;

use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, FieldDescriptorProto,
};

use super::{Messages, PACKAGE};

/// The messages where the tree stops.
const STOPS: [&str; 4] = ["ValueRef", "TableRef", "Table", "Encoding"];

/// The composition nodes, each with its structural fields: its children, its partition, and what
/// a composition sets, such as the direction of a concat. Their other fields are the options of
/// the compositions.
pub(super) const COMPOSITION_NODES: [(&str, &[&str]); 3] = [
    ("Plot", &["partition", "cartesian", "polar", "geo"]),
    ("Concat", &["direction", "children", "partition"]),
    ("Wrap", &["children", "partition"]),
];

/// The layer of the mark functions, with the fields that a mark call takes: its id and its
/// classes. Its other fields are its table, its partition, and its mark, which the call builds, its
/// tools, which the call places from the tools that it declares, and its widgets. The layers of the
/// other coordinate systems have the same options.
pub(super) const MARK_LAYER: (&str, &[&str]) = ("CartesianLayer", &["id", "classes"]);

/// The full names of the messages of the option tree, in the order that a walk from the roots
/// finds them.
pub(super) fn option_tree(messages: &Messages) -> Vec<String> {
    let encoding = format!("{PACKAGE}.Encoding");
    let mut roots: Vec<String> = Vec::new();
    if let Some((_, _, message)) = messages.get(&encoding) {
        let kind = (message.oneof_decl.iter()).position(|oneof| oneof.name() == "kind");
        for field in &message.field {
            let member_of_kind = kind.is_some() && field.oneof_index.map(|i| i as usize) == kind;
            if !member_of_kind && field.name() != "conditional" {
                roots.extend(message_type(field));
            }
        }
    }
    for (_, _, layer) in &messages.all {
        let Some(mark) = (layer.oneof_decl.iter()).position(|oneof| oneof.name() == "mark") else {
            continue;
        };
        for member in (layer.field.iter()).filter(|field| field.oneof_index == Some(mark as i32)) {
            roots.push(member.type_name().to_string());
        }
    }
    for plot in ["CartesianPlot", "PolarPlot", "GeoPlot"] {
        if let Some((_, _, message)) = messages.get(&format!("{PACKAGE}.{plot}")) {
            let options = message.field.iter().find(|field| field.name() == "options");
            roots.extend(options.and_then(message_type));
        }
    }
    let mut tree: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    // The composition nodes are members of the tree, and the walk follows only their options.
    for (node, structural) in COMPOSITION_NODES {
        let name = format!("{PACKAGE}.{node}");
        let Some((_, _, message)) = messages.get(&name) else {
            continue;
        };
        for field in (message.field.iter()).filter(|field| !structural.contains(&field.name())) {
            roots.extend(message_type(field));
        }
        seen.insert(name.clone());
        tree.push(name);
    }
    // The layer of the mark functions is a member of the tree, and its options are scalars.
    let layer = format!("{PACKAGE}.{}", MARK_LAYER.0);
    seen.insert(layer.clone());
    tree.push(layer);
    while let Some(name) = roots.pop() {
        if !seen.insert(name.clone()) || stops(&name) {
            continue;
        }
        let Some((_, _, message)) = messages.get(&name) else {
            continue;
        };
        tree.push(name);
        roots.extend(message.field.iter().filter_map(message_type));
    }
    tree
}

/// The fields that are not options of a message of the tree or of `Encoding`, by the message's
/// full name and the field's name: the structural fields of the composition nodes, the column of a
/// facet, which the composition that builds a facet sets from its key, the members of
/// `Encoding.kind`, which `scaled` and `direct` set, and `Encoding.conditional`.
pub(super) fn excluded(messages: &Messages) -> BTreeSet<(String, String)> {
    let encoding = format!("{PACKAGE}.Encoding");
    let mut excluded = BTreeSet::new();
    for (node, structural) in COMPOSITION_NODES {
        for field in structural {
            excluded.insert((format!("{PACKAGE}.{node}"), field.to_string()));
        }
    }
    let layer = format!("{PACKAGE}.{}", MARK_LAYER.0);
    if let Some((_, _, message)) = messages.get(&layer) {
        for field in (message.field.iter()).filter(|field| !MARK_LAYER.1.contains(&field.name())) {
            excluded.insert((layer.clone(), field.name().to_string()));
        }
    }
    excluded.insert((format!("{PACKAGE}.Facet"), "column".to_string()));
    if let Some((_, _, message)) = messages.get(&encoding) {
        let kind = (message.oneof_decl.iter()).position(|oneof| oneof.name() == "kind");
        for field in &message.field {
            let member_of_kind = kind.is_some() && field.oneof_index.map(|i| i as usize) == kind;
            if member_of_kind || field.name() == "conditional" {
                excluded.insert((encoding.clone(), field.name().to_string()));
            }
        }
    }
    excluded
}

/// The full names of the mark messages, the members of the `mark` oneof of each layer.
pub(super) fn marks(messages: &Messages) -> BTreeSet<String> {
    let mut marks = BTreeSet::new();
    for (_, _, layer) in &messages.all {
        let Some(mark) = (layer.oneof_decl.iter()).position(|oneof| oneof.name() == "mark") else {
            continue;
        };
        for member in (layer.field.iter()).filter(|field| field.oneof_index == Some(mark as i32)) {
            marks.insert(member.type_name().to_string());
        }
    }
    marks
}

/// Whether the tree stops at the message `name`.
fn stops(name: &str) -> bool {
    STOPS.iter().any(|stop| name == format!("{PACKAGE}.{stop}"))
}

/// The type of `field` when it is a message.
fn message_type(field: &FieldDescriptorProto) -> Option<String> {
    (field.r#type() == Type::Message).then(|| field.type_name().to_string())
}

/// The kind of SQL argument that a scalar member of a oneof takes, which two members of one oneof
/// do not share.
fn argument_kind(field: &FieldDescriptorProto) -> Option<&'static str> {
    if field.label() == Label::Repeated {
        return Some("array");
    }
    Some(match field.r#type() {
        Type::Bool => "Boolean",
        Type::Int32
        | Type::Int64
        | Type::Uint32
        | Type::Uint64
        | Type::Sint32
        | Type::Sint64
        | Type::Fixed32
        | Type::Fixed64
        | Type::Sfixed32
        | Type::Sfixed64 => "integer",
        Type::Float | Type::Double => "floating-point",
        Type::String | Type::Enum => "string",
        Type::Bytes => "bytes",
        Type::Message if field.type_name() == format!("{PACKAGE}.ValueRef") => "ValueRef",
        _ => return None,
    })
}

/// Checks the laws of the oneofs of the option tree and its message fields.
pub(super) fn check(messages: &Messages, tree: &[String]) -> Result<(), String> {
    let has_fields = |name: &str| {
        messages
            .get(name)
            .is_some_and(|(_, _, message)| !message.field.is_empty())
    };
    for name in tree {
        let (_, _, message) = messages.get(name).expect("a message of the tree");
        let short = message.name();
        for (index, oneof) in message.oneof_decl.iter().enumerate() {
            let members: Vec<&FieldDescriptorProto> = (message.field.iter())
                .filter(|field| field.oneof_index == Some(index as i32) && !field.proto3_optional())
                .collect();
            if members.is_empty() {
                continue;
            }
            check_oneof(short, oneof.name(), &members, &has_fields)?;
        }
        for field in message_fields(message) {
            let type_name = field.type_name();
            if !stops(type_name) && !has_fields(type_name) {
                return Err(format!(
                    "the option `{short}.{}` has the message `{}` without fields, which no \
                     argument sets outside a oneof",
                    field.name(),
                    type_name.rsplit('.').next().unwrap_or(type_name)
                ));
            }
        }
    }
    Ok(())
}

/// The message fields of `message` outside its oneofs.
fn message_fields(message: &DescriptorProto) -> impl Iterator<Item = &FieldDescriptorProto> {
    (message.field.iter()).filter(|field| {
        field.r#type() == Type::Message && (field.oneof_index.is_none() || field.proto3_optional())
    })
}

/// Checks that the argument of the oneof `oneof` of the message `message` picks at most one of
/// `members`.
fn check_oneof(
    message: &str,
    oneof: &str,
    members: &[&FieldDescriptorProto],
    has_fields: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    let at = format!("`{message}.{oneof}`");
    let mut types: Vec<(&str, &str)> = Vec::new();
    let mut kinds: Vec<(&str, &str)> = Vec::new();
    for member in members {
        let name = member.name();
        match argument_kind(member) {
            Some(kind) => {
                if let Some((other, _)) = kinds.iter().find(|(_, known)| *known == kind) {
                    return Err(format!(
                        "the members `{other}` and `{name}` of the oneof {at} both take a \
                         {kind} argument, so an argument cannot pick one"
                    ));
                }
                kinds.push((name, kind));
            }
            None if has_fields(member.type_name()) => {
                let type_name = member.type_name();
                if let Some((other, _)) = types.iter().find(|(_, known)| *known == type_name) {
                    return Err(format!(
                        "the members `{other}` and `{name}` of the oneof {at} have the message \
                         `{}`, so a fragment cannot pick one",
                        type_name.rsplit('.').next().unwrap_or(type_name)
                    ));
                }
                types.push((name, type_name));
            }
            // A member without fields is picked by its name.
            None => {}
        }
    }
    let takes_strings = kinds
        .iter()
        .find(|(_, kind)| matches!(*kind, "string" | "ValueRef"));
    let fieldless = members
        .iter()
        .find(|member| argument_kind(member).is_none() && !has_fields(member.type_name()));
    if let (Some((other, kind)), Some(member)) = (takes_strings, fieldless) {
        return Err(format!(
            "the member `{}` of the oneof {at} has a message without fields, which its name picks, \
             and the member `{other}` takes a {kind} argument, which a name would be",
            member.name()
        ));
    }
    Ok(())
}
