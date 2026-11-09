//! The table of properties in README.md, which this test renders from the property table of the
//! crate. When they differ, the test writes the rendered table to `tests/output/properties.md`,
//! which a change to the properties copies into README.md between the `properties` markers.

use std::{fs, path::Path};

use avenger_style::{values::Syntax, vocab::Element, Property};

const START: &str = "<!-- properties -->\n";
const END: &str = "<!-- /properties -->";

fn bound(min: Option<f32>, max: Option<f32>) -> String {
    match (min, max) {
        (None, None) => String::new(),
        (min, max) => format!(
            " [{},{}]",
            min.map_or("-∞".to_string(), |min| min.to_string()),
            max.map_or("∞".to_string(), |max| max.to_string()),
        ),
    }
}

fn syntax(syntax: &Syntax) -> String {
    match syntax {
        Syntax::Length { min } => format!("<length{}>", bound(*min, None)),
        Syntax::Number { min, max } => format!("<number{}>", bound(*min, *max)),
        Syntax::Integer { min } => format!("<integer [{min},∞]>"),
        Syntax::Angle => "<angle>".to_string(),
        Syntax::Percentage => "<percentage>".to_string(),
        Syntax::Color => "<color>".to_string(),
        Syntax::Boolean => "true | false".to_string(),
        Syntax::Keywords(keywords) => keywords.join(" | "),
        Syntax::KeywordList(keywords) => format!("[{}]+", keywords.join(" | ")),
        Syntax::String => "<string>".to_string(),
        Syntax::FontFamilies => "<family-name>#".to_string(),
        Syntax::FontWeight => "normal | bold | <number [1,1000]>".to_string(),
        Syntax::Ratio => "<ratio>".to_string(),
        Syntax::Scheme => "auto | <palette-name> <integer>? reverse?".to_string(),
        Syntax::Shape => "<shape>".to_string(),
        Syntax::Pattern => "none | <pattern>".to_string(),
        Syntax::OutputList => "<output>#".to_string(),
        Syntax::OutputItem => "<output>".to_string(),
        Syntax::Dash => "<length>+".to_string(),
        Syntax::Line => "<line-width> || <line-style> || <color>".to_string(),
        Syntax::Repeat(inner, min, max) => format!("{}{{{min},{max}}}", self::syntax(inner)),
        Syntax::Any(alternatives) => (alternatives.iter())
            .map(self::syntax)
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

fn elements(property: Property) -> String {
    let nodes: Vec<Element> = (Element::ALL.iter())
        .copied()
        // Every property applies to `other`, the variant of its parent, so the table leaves it out.
        .filter(|element| !element.is_virtual() && *element != Element::Other)
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
            code(&syntax(property.syntax())),
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
