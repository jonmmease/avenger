//! The table of properties in README.md, which this test renders from the property table of the
//! crate. When they differ, the test writes the rendered table to `tests/output/properties.md`,
//! which a change to the properties copies into README.md between the `properties` markers.

use std::{fs, path::Path};

use avenger_style::{vocab::Element, Property};

const START: &str = "<!-- properties -->\n";
const END: &str = "<!-- /properties -->";

fn elements(property: Property) -> String {
    let nodes: Vec<Element> = (Element::ALL.iter())
        .copied()
        // Every property applies to a variant of its parent, so the table leaves the variants out.
        .filter(|element| !element.is_variant())
        .collect();
    let elements = property.elements();
    if elements.len() == nodes.len() && nodes.iter().all(|node| elements.contains(node)) {
        return "every node".to_string();
    }
    (elements.iter())
        .map(|element| format!("`{element}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The cell text of `text` as code, with the pipes that a table cell escapes.
fn code(text: &str) -> String {
    format!("`{}`", text.replace('|', "\\|"))
}

fn table() -> String {
    let mut table = String::from(
        "| Property | Values | Initial | Inherited | Elements |\n|---|---|---|---|---|\n",
    );
    for property in Property::ALL {
        table.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            property.name(),
            code(&property.syntax().grammar()),
            code(property.initial()),
            match property.is_inherited() {
                true => "yes",
                false => "no",
            },
            elements(*property),
        ));
    }
    table
}

#[test]
fn the_readme_lists_every_property() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    let start = readme.find(START).expect("README.md has the start marker") + START.len();
    let end = readme.find(END).expect("README.md has the end marker");
    let expected = table();
    if readme[start..end] != expected {
        let output = root.join("tests/output");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("properties.md"), &expected).unwrap();
        panic!(
            "the table of properties in README.md is out of date; copy tests/output/properties.md \
             into README.md between the markers"
        );
    }
}
