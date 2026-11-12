//! The generation of `compositions.rs`, behind the `functions` feature: the table of the
//! composition functions, which compose charts into a layered plot, a concat, or a facet, with the
//! parameters of each, which are the options of its node, and the function that applies them.
//!
//! The options of a composition are the parameters of its node in the parameter model, without
//! the parameters that the composition sets itself: `layer` takes those of `Plot`, `hconcat` and
//! `vconcat` those of `Concat` without its `facet`, the facets of rows and columns those of
//! `Concat`, and `facet_wrap` those of `Wrap`.

use std::fmt::Write;

use super::{model::Model, parameters::builder, PACKAGE};

/// A composition: its name, its kind, the node that it builds, the parameters of the node that it
/// does not take, and its documentation.
struct Spec {
    name: &'static str,
    kind: &'static str,
    node: &'static str,
    omit: &'static [&'static str],
    doc: &'static str,
}

const SPECS: [Spec; 6] = [
    Spec {
        name: "layer",
        kind: "Layer",
        node: "Plot",
        omit: &[],
        doc:
            "The plot of the layers of a column of charts, whose roots are plots of one coordinate \
              system. The charts draw in their order, each over the ones before it, and their \
              layers share every channel by default. The options are those of the plot, and an \
              option that a layered plot sets applies when the call does not set it.",
    },
    Spec {
        name: "hconcat",
        kind: "HConcat",
        node: "Concat",
        omit: &["facet"],
        doc: "A concat of a column of charts side by side, from left to right, in their order. \
              The options are those of the concat.",
    },
    Spec {
        name: "vconcat",
        kind: "VConcat",
        node: "Concat",
        omit: &["facet"],
        doc: "A concat of a column of charts one above another, from top to bottom, in their \
              order. The options are those of the concat.",
    },
    Spec {
        name: "facet_row",
        kind: "FacetRow",
        node: "Concat",
        omit: &[],
        doc: "A facet of a column of charts by a key: one row for each value of the key, in a \
              column, each labeled with its value. The values are ascending with NULL last, or in \
              the order of their first rows. The options are those of the concat, and `facet` \
              sets the labels and the title of the facet, whose column is the key's.",
    },
    Spec {
        name: "facet_col",
        kind: "FacetCol",
        node: "Concat",
        omit: &[],
        doc: "A facet of a column of charts by a key: one column for each value of the key, in a \
              row, each labeled with its value. The values are ascending with NULL last, or in \
              the order of their first rows. The options are those of the concat, and `facet` \
              sets the labels and the title of the facet, whose column is the key's.",
    },
    Spec {
        name: "facet_wrap",
        kind: "FacetWrap",
        node: "Wrap",
        omit: &[],
        doc: "A facet of a column of charts by a key in rows that fill the width, each chart \
              labeled with its value. The values are ascending with NULL last, or in the order of \
              their first rows. The options are those of the wrap, and `facet` sets the labels and \
              the title of the facet, whose column is the key's.",
    },
];

/// The code of `compositions.rs`.
pub(super) fn compositions(model: &Model) -> Result<String, String> {
    let mut code = String::new();
    for spec in &SPECS {
        let name = format!("{PACKAGE}.{}", spec.node);
        let message = model
            .message(&name)
            .ok_or(format!("`{name}` has a model"))?;
        let node_params = format!("PARAMS_{}", message.function.to_uppercase());
        let indices: Vec<usize> = (message.params.iter().enumerate())
            .filter(|(_, param)| !spec.omit.contains(&param.name.as_str()))
            .map(|(index, _)| index)
            .collect();
        for omitted in spec.omit {
            if !message.params.iter().any(|param| param.name == *omitted) {
                return Err(format!("`{}` has no parameter `{omitted}`", spec.node));
            }
        }
        let own = format!("PARAMS_{}", spec.name.to_uppercase());
        let items: Vec<String> = (indices.iter())
            .map(|index| format!("{node_params}[{index}]"))
            .collect();
        writeln!(
            code,
            "pub(crate) const {own}: &[Param] = &[{}];\n",
            items.join(", ")
        )
        .unwrap();
        let builder = builder(&name);
        let indices: Vec<String> = indices.iter().map(usize::to_string).collect();
        writeln!(
            code,
            "pub(crate) fn apply_all_{}(mut builder: {builder}, args: &[Option<Arg>]) -> Result<{builder}, ArgError> {{\n    const NODE: &[usize] = &[{}];\n    for (index, arg) in args.iter().enumerate() {{\n        let param = &{own}[index];\n        match arg {{\n            Some(arg) => builder = apply_{}(builder, NODE[index], arg).map_err(|error| error.within(param.name))?,\n            None if param.required => return Err(ArgError::required(param.name)),\n            None => {{}}\n        }}\n    }}\n    Ok(builder)\n}}\n",
            spec.name,
            indices.join(", "),
            message.function
        )
        .unwrap();
    }
    code.push_str("pub(crate) static COMPOSITIONS: &[Composition] = &[\n");
    for spec in &SPECS {
        let doc = spec.doc.split_whitespace().collect::<Vec<_>>().join(" ");
        writeln!(
            code,
            "    Composition {{ name: {:?}, kind: CompositionKind::{}, doc: {doc:?}, params: PARAMS_{} }},",
            spec.name,
            spec.kind,
            spec.name.to_uppercase()
        )
        .unwrap();
    }
    code.push_str("];\n");
    Ok(code)
}
