//! The names of the format follow the style tree: a style field is `[part_]leaf`, where the part
//! is an element below its holder's element and the leaf is a property of the stylesheet
//! vocabulary that the part, or the holder, takes, and a word that names an element starts a name
//! only when the name is a setting of that part. The stylesheet vocabulary follows the same rule:
//! no property starts with the name of an element. A member of a oneof also follows the rule by
//! the oneof's name, the name that avenger-sql writes, as `range` for the `values` and the `scheme`
//! of a scale.
//!
//! The elements that each message styles come from `STYLE_FIELDS`, and the parts below each
//! element from `Element::parts`, which the style tree asserts as it adds nodes.

use std::collections::{BTreeMap, BTreeSet};

use avenger_chart_eval::STYLE_FIELDS;
use avenger_style::{Element, Property, Shorthand};
use prost::Message;

/// The reason of the `filled` of each symbol mark.
const FILLED: &str = "a setting of the mark that chooses between filling and outlining with \
                      `color`, written as `fill` and `stroke`";

/// The style fields that are not a leaf of their node by name, with the reason.
const EXCEPTIONS: &[(&str, &str, &str)] = &[
    ("SymbolMark", "filled", FILLED),
    ("PolarSymbolMark", "filled", FILLED),
    ("GeoSymbolMark", "filled", FILLED),
];

/// The messages outside the style tree whose fields start with the name of an element, with the
/// element that they stand for and the reason.
const OUTSIDE: &[(&str, Element, &str)] = &[(
    "PartRules",
    Element::Plot,
    "the rules of the layout name the parts of a plot, as `axis_labels`",
)];

/// The fields of each message of the format, with its oneofs, by message name.
fn fields() -> BTreeMap<String, BTreeSet<String>> {
    let set =
        prost_types::FileDescriptorSet::decode(avenger_chart::proto::FILE_DESCRIPTOR_SET).unwrap();
    let mut fields: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in (set.file.iter()).filter(|file| file.package() == "avenger.chart.v1") {
        for message in &file.message_type {
            let names = fields.entry(message.name().to_string()).or_default();
            names.extend(message.field.iter().map(|field| field.name().to_string()));
            names.extend(
                message
                    .oneof_decl
                    .iter()
                    .map(|oneof| oneof.name().to_string()),
            );
        }
    }
    fields
}

/// The oneof of each field that is a member of one, by message and field. A proto3 `optional`
/// field is the only member of a synthetic oneof, which is not one.
fn oneofs() -> BTreeMap<(String, String), String> {
    let set =
        prost_types::FileDescriptorSet::decode(avenger_chart::proto::FILE_DESCRIPTOR_SET).unwrap();
    let mut oneofs = BTreeMap::new();
    for file in (set.file.iter()).filter(|file| file.package() == "avenger.chart.v1") {
        for message in &file.message_type {
            for field in &message.field {
                let (Some(index), false) = (field.oneof_index, field.proto3_optional()) else {
                    continue;
                };
                let oneof = message.oneof_decl[index as usize].name().to_string();
                let key = (message.name().to_string(), field.name().to_string());
                oneofs.insert(key, oneof);
            }
        }
    }
    oneofs
}

/// The oneof of `field` of `message`, when it is a member of one.
fn oneof_of<'a>(
    oneofs: &'a BTreeMap<(String, String), String>,
    message: &str,
    field: &str,
) -> Option<&'a str> {
    let key = (message.to_string(), field.to_string());
    oneofs.get(&key).map(String::as_str)
}

/// Whether `field` of `message` follows the rule by its own name or by its oneof's.
fn follows_by_name(
    oneofs: &BTreeMap<(String, String), String>,
    message: &str,
    elements: &[Element],
    field: &str,
) -> bool {
    follows(elements, field)
        || oneof_of(oneofs, message, field).is_some_and(|oneof| follows(elements, oneof))
}

/// The elements that `message` stands for: the nodes that `STYLE_FIELDS` says it styles, or the
/// element of a message outside the style tree.
fn holders(message: &str) -> Option<Vec<Element>> {
    let styled = (STYLE_FIELDS.iter()).find(|(name, _, _)| *name == message);
    let outside = (OUTSIDE.iter()).find(|(name, _, _)| *name == message);
    match (styled, outside) {
        (Some((_, elements, _)), _) => Some(elements.to_vec()),
        (None, Some((_, element, _))) => Some(vec![*element]),
        (None, None) => None,
    }
}

/// The parts below any of `elements`.
fn parts(elements: &[Element]) -> Vec<Element> {
    (elements.iter())
        .flat_map(|element| element.parts())
        .copied()
        .collect()
}

/// Whether the leaf `leaf`, a property or a shorthand in snake case, applies to `element`.
fn takes(element: Element, leaf: &str) -> bool {
    let name = leaf.replace('_', "-");
    let property = (Property::ALL.iter()).find(|property| property.name() == name);
    if let Some(property) = property {
        return property.applies_to(element);
    }
    Shorthand::from_name(&name).is_some_and(|shorthand| {
        (shorthand.longhands().iter()).any(|longhand| longhand.applies_to(element))
    })
}

/// The part that a field names, in the singular or in Vega-Lite's plural.
fn named_part(field: &str, parts: &[Element]) -> Option<Element> {
    let named = |name: &str| (parts.iter().copied()).find(|part| part.name() == name);
    named(field)
        .or_else(|| field.strip_suffix("es").and_then(named))
        .or_else(|| field.strip_suffix('s').and_then(named))
}

/// Whether `field` of a message that styles `elements` follows the rule: a leaf that one of the
/// elements takes, a part below them, or a part's name and a leaf that the part takes.
fn follows(elements: &[Element], field: &str) -> bool {
    let parts = parts(elements);
    let leaf = elements.iter().any(|element| takes(*element, field));
    let part = named_part(field, &parts).is_some();
    let prefixed = (field.split_once('_')).is_some_and(|(prefix, leaf)| {
        (parts.iter()).any(|part| part.name() == prefix && takes(*part, leaf))
    });
    leaf || part || prefixed
}

fn excepted(message: &str, field: &str) -> bool {
    (EXCEPTIONS.iter()).any(|(name, exception, _)| *name == message && *exception == field)
}

#[test]
fn no_property_starts_with_the_name_of_an_element() {
    let elements: BTreeSet<&str> = Element::ALL.iter().map(|element| element.name()).collect();
    let starting: Vec<&str> = (Property::ALL.iter())
        .map(|property| property.name())
        .filter(|name| (name.split_once('-')).is_some_and(|(first, _)| elements.contains(first)))
        .collect();
    assert!(
        starting.is_empty(),
        "properties that start with an element's name, where a property of a part is a property \
         of the part's element: {starting:?}"
    );
}

#[test]
fn every_style_field_is_a_part_and_a_leaf_of_its_node() {
    let oneofs = oneofs();
    let mut wrong = Vec::new();
    for (message, elements, names) in STYLE_FIELDS {
        for field in names.iter() {
            if !excepted(message, field) && !follows_by_name(&oneofs, message, elements, field) {
                wrong.push(format!("{message}.{field}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "style fields that are neither a leaf of their node, a part below it, nor a part's prefix \
         and a leaf of the part: {wrong:?}"
    );
}

#[test]
fn a_word_that_names_an_element_starts_only_the_settings_of_that_part() {
    let mut wrong = Vec::new();
    for (message, names) in fields() {
        let parts = holders(&message).map(|holders| parts(&holders));
        for field in &names {
            let Some((first, _)) = field.split_once('_') else {
                continue;
            };
            if Element::from_name(first).is_none() {
                continue;
            }
            // A part of the holder, where a message outside the style tree has no parts.
            let part = (parts.iter().flatten()).any(|part| part.name() == first);
            if !part {
                wrong.push(format!("{message}.{field}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "names whose first word names an element that is not a part of their holder: {wrong:?}"
    );
}

/// Each exception names a field that exists and that the rules reject, so a stale exception
/// fails.
#[test]
fn the_exceptions_name_fields_that_the_rules_reject() {
    let fields = fields();
    let oneofs = oneofs();
    for (message, field, _) in EXCEPTIONS {
        assert!(
            fields
                .get(*message)
                .is_some_and(|names| names.contains(*field)),
            "no field `{message}.{field}`"
        );
        let elements = holders(message).unwrap_or_else(|| panic!("`{message}` styles no node"));
        assert!(
            !follows_by_name(&oneofs, message, &elements, field),
            "`{message}.{field}` follows the rules, so it needs no exception"
        );
    }
    for (message, _, _) in OUTSIDE {
        assert!(fields.contains_key(*message), "no message `{message}`");
        assert!(
            (STYLE_FIELDS.iter()).all(|(name, _, _)| name != message),
            "`{message}` has style fields, so `STYLE_FIELDS` gives its elements"
        );
    }
}

/// The rules reject the names that the style tree's renames replaced, so a stale name fails.
#[test]
fn the_rules_reject_the_names_that_the_renames_replaced() {
    // A leaf that its part does not take.
    assert!(!follows(&[Element::Axis], "tick_size"));
    assert!(!follows(&[Element::Graticule], "grid_width"));
    assert!(!follows(&[Element::Axis], "rule_width"));
    // A part that the holder does not have, as the colorbar's `axis_gap` named.
    assert!(!follows(&[Element::Legend], "axis_gap"));
    // The names that replaced them.
    assert!(follows(&[Element::Axis], "tick_length"));
    assert!(follows(&[Element::Graticule], "grid_stroke_width"));
    assert!(follows(&[Element::Axis], "rule_stroke"));
    assert!(follows(&[Element::Legend], "ramp_margin"));
    // A member of a oneof follows by the oneof's name, as the aspect of a geo plot, where the
    // member `free` of the oneof `aspect` did not.
    let oneofs = oneofs();
    assert_eq!(oneof_of(&oneofs, "Scale", "values"), Some("range"));
    assert!(follows_by_name(&oneofs, "Geo", &[Element::Plot], "none"));
    assert!(!follows(&[Element::Plot], "free"));
    assert!(!follows(&[Element::Plot], "aspect"));
}
