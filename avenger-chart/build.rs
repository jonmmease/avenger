//! Generates the prost messages of package `avenger.chart.v1` from the files in
//! `proto/avenger/chart/v1/` with the vendored `protoc`, so that a build needs no system `protoc`.
//!
//! It fails when a field of a mark breaks a law of the format: an `Encoding` field has the name of
//! a `Channel` value, lowercase and without the prefix, and the number 100 plus that value, and
//! every other field, a property, has a number below 100.

use std::path::PathBuf;

use prost::Message;
use prost_types::{field_descriptor_proto::Type, DescriptorProto, FileDescriptorSet};

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
    check_marks(&set)?;
    Ok(())
}

/// Checks the laws of the fields of each mark: each member of the `mark` oneof of a layer.
fn check_marks(set: &FileDescriptorSet) -> Result<(), String> {
    let messages: Vec<(String, &DescriptorProto)> = set
        .file
        .iter()
        .flat_map(|file| &file.message_type)
        .map(|message| (format!("{PACKAGE}.{}", message.name()), message))
        .collect();
    let message = |name: &str| {
        (messages.iter())
            .find(|(full, _)| full == name)
            .map(|(_, message)| *message)
    };
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
    // The layers: the messages with a `mark` oneof, with the index of the oneof.
    let layers: Vec<(&DescriptorProto, usize)> = (messages.iter())
        .filter_map(|(_, message)| {
            let oneof = (message.oneof_decl.iter()).position(|oneof| oneof.name() == "mark")?;
            Some((*message, oneof))
        })
        .collect();
    if layers.is_empty() {
        return Err("the package has layers, messages with a `mark` oneof".to_string());
    }
    for (layer, oneof) in layers {
        for member in (layer.field.iter()).filter(|field| field.oneof_index == Some(oneof as i32)) {
            let message = message(member.type_name())
                .ok_or_else(|| format!("`{}.{}` is a message", layer.name(), member.name()))?;
            for field in &message.field {
                let at = format!("`{}.{}`", message.name(), field.name());
                if field.r#type() == Type::Message
                    && field.type_name() == format!("{PACKAGE}.Encoding")
                {
                    let (_, value) = (channels.iter())
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
                } else if field.number() >= 100 {
                    return Err(format!("the property {at} has a number below 100"));
                }
            }
        }
    }
    Ok(())
}
