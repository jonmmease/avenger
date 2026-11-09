//! The text style of some labels of a guide apart from the rest, such as the label of the Other
//! group of a lump, which the label's markup carries.

use avenger_common::types::{FontWeight, TextSyntaxMode};
use avenger_typst_label::escape_text;

/// How the text of a label differs from the other labels of its guide: each field that is set
/// replaces the guide's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelVariant {
    pub color: Option<[f32; 4]>,
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    pub font_weight: Option<FontWeight>,
    pub italic: Option<bool>,
}

impl LabelVariant {
    /// Whether the variant changes nothing.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// `markup` in the text style of the variant: a Typst `text` call with the variant's fields.
    pub fn wrap(&self, markup: &str) -> String {
        let mut args = Vec::new();
        if let Some(color) = self.color {
            let [r, g, b, a] = color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            args.push(format!("fill: rgb({r}, {g}, {b}, {a})"));
        }
        if let Some(family) = &self.font_family {
            let family = family.replace('\\', "\\\\").replace('"', "\\\"");
            args.push(format!("font: \"{family}\""));
        }
        if let Some(size) = self.font_size {
            args.push(format!("size: {size}pt"));
        }
        if let Some(weight) = self.font_weight {
            args.push(format!("weight: {}", weight.0));
        }
        if let Some(italic) = self.italic {
            let style = if italic { "italic" } else { "normal" };
            args.push(format!("style: \"{style}\""));
        }
        format!("#text({})[{markup}]", args.join(", "))
    }
}

/// `text`, labels in `syntax`, with each label that `variants` holds a variant for in its variant.
/// When a variant changes anything, the labels become Typst markup, with plain text escaped.
pub fn vary_labels(
    text: Vec<String>,
    syntax: TextSyntaxMode,
    variants: &[Option<LabelVariant>],
) -> (Vec<String>, TextSyntaxMode) {
    let varies = |index: usize| {
        (variants.get(index))
            .and_then(Option::as_ref)
            .filter(|variant| !variant.is_empty())
    };
    if !(0..text.len()).any(|index| varies(index).is_some()) {
        return (text, syntax);
    }
    let text = (text.into_iter().enumerate())
        .map(|(index, text)| {
            let markup = match syntax {
                TextSyntaxMode::Plain => escape_text(&text),
                _ => text,
            };
            match varies(index) {
                Some(variant) => variant.wrap(&markup),
                None => markup,
            }
        })
        .collect();
    (text, TextSyntaxMode::TypstMarkup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variant_wraps_its_labels_and_escapes_the_rest() {
        let italic = LabelVariant {
            italic: Some(true),
            color: Some([1.0, 0.0, 0.0, 1.0]),
            ..Default::default()
        };
        let (text, syntax) = vary_labels(
            vec!["a*".into(), "Rest".into()],
            TextSyntaxMode::Plain,
            &[None, Some(italic)],
        );
        assert_eq!(syntax, TextSyntaxMode::TypstMarkup);
        assert_eq!(
            text,
            [
                r"a\*",
                r#"#text(fill: rgb(255, 0, 0, 255), style: "italic")[Rest]"#
            ]
        );
        // An empty variant leaves the labels as they are.
        let (text, syntax) = vary_labels(
            vec!["a*".into()],
            TextSyntaxMode::Plain,
            &[Some(LabelVariant::default())],
        );
        assert_eq!(
            (text, syntax),
            (vec!["a*".to_string()], TextSyntaxMode::Plain)
        );
    }
}
