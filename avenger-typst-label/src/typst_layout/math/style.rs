fn style_default_math_char(ch: char) -> char {
    if ch.is_ascii_alphabetic() || is_lower_greek_math_char(ch) || matches!(ch, 'ı' | 'ȷ' | 'ħ')
    {
        to_math_italic(ch)
    } else {
        ch
    }
}

fn is_lower_greek_math_char(ch: char) -> bool {
    matches!(ch, 'α'..='ω' | '∂' | 'ϵ' | 'ϑ' | 'ϰ' | 'ϕ' | 'ϱ' | 'ϖ')
}

fn to_math_italic(ch: char) -> char {
    let delta = match ch {
        'h' => 0x20A6,
        'ħ' => 0x1FE8,
        'A'..='Z' => 0x1D3F3,
        'a'..='z' => 0x1D3ED,
        'ı' => 0x1D573,
        'ȷ' => 0x1D46E,
        'Α'..='Ρ' => 0x1D351,
        'ϴ' => 0x1D2FF,
        'Σ'..='Ω' => 0x1D351,
        '∇' => 0x1B4F4,
        'α'..='ω' => 0x1D34B,
        '∂' => 0x1B513,
        'ϵ' => 0x1D321,
        'ϑ' => 0x1D346,
        'ϰ' => 0x1D328,
        'ϕ' => 0x1D344,
        'ϱ' => 0x1D329,
        'ϖ' => 0x1D345,
        _ => return ch,
    };
    std::char::from_u32((ch as u32) + delta).unwrap_or(ch)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MathStyleSelection {
    variant: Option<MathVariant>,
    bold: bool,
    italic: Option<bool>,
}

impl MathStyleSelection {
    fn from_call_name(name: &str) -> Option<Self> {
        let mut selection = Self::default();
        match name {
            "bold" => selection.bold = true,
            "upright" => selection.italic = Some(false),
            "italic" => selection.italic = Some(true),
            "serif" => selection.variant = Some(MathVariant::Plain),
            "sans" => selection.variant = Some(MathVariant::SansSerif),
            "cal" => selection.variant = Some(MathVariant::Chancery),
            "scr" => selection.variant = Some(MathVariant::Roundhand),
            "frak" => selection.variant = Some(MathVariant::Fraktur),
            "mono" => selection.variant = Some(MathVariant::Monospace),
            "bb" => selection.variant = Some(MathVariant::DoubleStruck),
            _ => return None,
        }
        Some(selection)
    }

    fn compose(self, nested: Self) -> Self {
        Self {
            variant: nested.variant.or(self.variant),
            bold: self.bold || nested.bold,
            italic: nested.italic.or(self.italic),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathVariant {
    Plain,
    Fraktur,
    SansSerif,
    Monospace,
    DoubleStruck,
    Chancery,
    Roundhand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathAlphabetStyle {
    Plain,
    Bold,
    Italic,
    BoldItalic,
    Fraktur,
    BoldFraktur,
    SansSerif,
    SansSerifBold,
    SansSerifItalic,
    SansSerifBoldItalic,
    Monospace,
    DoubleStruck,
    DoubleStruckItalic,
    Chancery,
    BoldChancery,
    Roundhand,
    BoldRoundhand,
    Hebrew,
}

impl MathAlphabetStyle {
    fn select(ch: char, selection: MathStyleSelection) -> Self {
        use MathAlphabetStyle::*;

        match (
            selection.variant.unwrap_or(MathVariant::Plain),
            selection.bold,
            selection.italic,
        ) {
            (MathVariant::SansSerif, false, Some(false)) if ch.is_ascii_alphabetic() => SansSerif,
            (MathVariant::SansSerif, false, _) if ch.is_ascii_alphabetic() => SansSerifItalic,
            (MathVariant::SansSerif, true, Some(false)) if ch.is_ascii_alphabetic() => {
                SansSerifBold
            }
            (MathVariant::SansSerif, true, _) if ch.is_ascii_alphabetic() => SansSerifBoldItalic,
            (MathVariant::SansSerif, false, _) if ch.is_ascii_digit() => SansSerif,
            (MathVariant::SansSerif, true, _) if ch.is_ascii_digit() => SansSerifBold,
            (MathVariant::SansSerif, _, Some(false)) if is_greek_math_char(ch) => SansSerifBold,
            (MathVariant::SansSerif, _, Some(true)) if is_greek_math_char(ch) => {
                SansSerifBoldItalic
            }
            (MathVariant::SansSerif, _, None) if is_upper_greek_math_char(ch) => SansSerifBold,
            (MathVariant::SansSerif, _, None) if is_lower_greek_math_char(ch) => {
                SansSerifBoldItalic
            }
            (MathVariant::Fraktur, false, _) if ch.is_ascii_alphabetic() => Fraktur,
            (MathVariant::Fraktur, true, _) if ch.is_ascii_alphabetic() => BoldFraktur,
            (MathVariant::Monospace, _, _) if ch.is_ascii_digit() || ch.is_ascii_alphabetic() => {
                Monospace
            }
            (MathVariant::DoubleStruck, _, Some(true))
                if matches!(ch, 'D' | 'd' | 'e' | 'i' | 'j') =>
            {
                DoubleStruckItalic
            }
            (MathVariant::DoubleStruck, _, _)
                if ch.is_ascii_digit()
                    || ch.is_ascii_alphabetic()
                    || matches!(ch, '∑' | 'Γ' | 'Π' | 'γ' | 'π') =>
            {
                DoubleStruck
            }
            (MathVariant::Chancery, false, _) if ch.is_ascii_alphabetic() => Chancery,
            (MathVariant::Chancery, true, _) if ch.is_ascii_alphabetic() => BoldChancery,
            (MathVariant::Roundhand, false, _) if ch.is_ascii_alphabetic() => Roundhand,
            (MathVariant::Roundhand, true, _) if ch.is_ascii_alphabetic() => BoldRoundhand,
            (_, false, Some(true)) if ch.is_ascii_alphabetic() || is_greek_math_char(ch) => Italic,
            (_, false, None) if ch.is_ascii_alphabetic() || is_lower_greek_math_char(ch) => Italic,
            (_, true, Some(false)) if ch.is_ascii_alphabetic() || is_greek_math_char(ch) => Bold,
            (_, true, Some(true)) if ch.is_ascii_alphabetic() || is_greek_math_char(ch) => {
                BoldItalic
            }
            (_, true, None) if ch.is_ascii_alphabetic() || is_lower_greek_math_char(ch) => {
                BoldItalic
            }
            (_, true, None) if is_upper_greek_math_char(ch) => Bold,
            (_, true, _) if ch.is_ascii_digit() || matches!(ch, 'Ϝ' | 'ϝ') => Bold,
            (_, _, Some(true) | None) if matches!(ch, 'ı' | 'ȷ' | 'ħ') => Italic,
            (_, _, Some(true) | None) if is_hebrew_math_char(ch) => Hebrew,
            _ => Plain,
        }
    }
}

fn style_math_char(ch: char, style: MathAlphabetStyle) -> [char; 2] {
    use MathAlphabetStyle::*;
    match style {
        Plain => [ch, '\0'],
        Bold => [to_math_bold(ch), '\0'],
        Italic => [to_math_italic(ch), '\0'],
        BoldItalic => [to_math_bold_italic(ch), '\0'],
        Fraktur => [to_math_fraktur(ch), '\0'],
        BoldFraktur => [to_math_bold_fraktur(ch), '\0'],
        SansSerif => [to_math_sans_serif(ch), '\0'],
        SansSerifBold => [to_math_sans_serif_bold(ch), '\0'],
        SansSerifItalic => [to_math_sans_serif_italic(ch), '\0'],
        SansSerifBoldItalic => [to_math_sans_serif_bold_italic(ch), '\0'],
        Monospace => [to_math_monospace(ch), '\0'],
        DoubleStruck => [to_math_double_struck(ch), '\0'],
        DoubleStruckItalic => [to_math_double_struck_italic(ch), '\0'],
        Chancery => with_variation_selector(to_math_script(ch), '\u{fe00}', ch),
        BoldChancery => with_variation_selector(to_math_bold_script(ch), '\u{fe00}', ch),
        Roundhand => with_variation_selector(to_math_script(ch), '\u{fe01}', ch),
        BoldRoundhand => with_variation_selector(to_math_bold_script(ch), '\u{fe01}', ch),
        Hebrew => [to_math_hebrew(ch), '\0'],
    }
}

fn with_variation_selector(styled: char, selector: char, original: char) -> [char; 2] {
    if styled == original && !original.is_ascii_alphabetic() {
        [styled, '\0']
    } else {
        [styled, selector]
    }
}

fn is_greek_math_char(ch: char) -> bool {
    is_upper_greek_math_char(ch) || is_lower_greek_math_char(ch)
}

fn is_upper_greek_math_char(ch: char) -> bool {
    matches!(ch, 'Α'..='Ω' | '∇' | 'ϴ')
}

fn is_hebrew_math_char(ch: char) -> bool {
    matches!(ch, 'א'..='ד')
}

fn apply_math_delta(ch: char, delta: u32) -> char {
    std::char::from_u32((ch as u32) + delta).unwrap_or(ch)
}

fn to_math_bold(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D3BF,
        'a'..='z' => 0x1D3B9,
        'Α'..='Ρ' => 0x1D317,
        'ϴ' => 0x1D2C5,
        'Σ'..='Ω' => 0x1D317,
        '∇' => 0x1B4BA,
        'α'..='ω' => 0x1D311,
        '∂' => 0x1B4D9,
        'ϵ' => 0x1D2E7,
        'ϑ' => 0x1D30C,
        'ϰ' => 0x1D2EE,
        'ϕ' => 0x1D30A,
        'ϱ' => 0x1D2EF,
        'ϖ' => 0x1D30B,
        'Ϝ'..='ϝ' => 0x1D3EE,
        '0'..='9' => 0x1D79E,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_bold_italic(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D427,
        'a'..='z' => 0x1D421,
        'Α'..='Ρ' => 0x1D38B,
        'ϴ' => 0x1D339,
        'Σ'..='Ω' => 0x1D38B,
        '∇' => 0x1B52E,
        'α'..='ω' => 0x1D385,
        '∂' => 0x1B54D,
        'ϵ' => 0x1D35B,
        'ϑ' => 0x1D380,
        'ϰ' => 0x1D362,
        'ϕ' => 0x1D37E,
        'ϱ' => 0x1D363,
        'ϖ' => 0x1D37F,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_script(ch: char) -> char {
    let delta = match ch {
        'g' => 0x20A3,
        'H' => 0x20C3,
        'I' => 0x20C7,
        'L' => 0x20C6,
        'R' => 0x20C9,
        'B' => 0x20EA,
        'e' => 0x20CA,
        'E'..='F' => 0x20EB,
        'M' => 0x20E6,
        'o' => 0x20C5,
        'A'..='Z' => 0x1D45B,
        'a'..='z' => 0x1D455,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_bold_script(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D48F,
        'a'..='z' => 0x1D489,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_fraktur(ch: char) -> char {
    let delta = match ch {
        'H' => 0x20C4,
        'I' => 0x20C8,
        'R' => 0x20CA,
        'Z' => 0x20CE,
        'C' => 0x20EA,
        'A'..='Z' => 0x1D4C3,
        'a'..='z' => 0x1D4BD,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_bold_fraktur(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D52B,
        'a'..='z' => 0x1D525,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_sans_serif(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D55F,
        'a'..='z' => 0x1D559,
        '0'..='9' => 0x1D7B2,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_sans_serif_bold(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D593,
        'a'..='z' => 0x1D58D,
        'Α'..='Ρ' => 0x1D3C5,
        'ϴ' => 0x1D373,
        'Σ'..='Ω' => 0x1D3C5,
        '∇' => 0x1B568,
        'α'..='ω' => 0x1D3BF,
        '∂' => 0x1B587,
        'ϵ' => 0x1D395,
        'ϑ' => 0x1D3BA,
        'ϰ' => 0x1D39C,
        'ϕ' => 0x1D3B8,
        'ϱ' => 0x1D39D,
        'ϖ' => 0x1D3B9,
        '0'..='9' => 0x1D7BC,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_sans_serif_italic(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D5C7,
        'a'..='z' => 0x1D5C1,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_sans_serif_bold_italic(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D5FB,
        'a'..='z' => 0x1D5F5,
        'Α'..='Ρ' => 0x1D3FF,
        'ϴ' => 0x1D3AD,
        'Σ'..='Ω' => 0x1D3FF,
        '∇' => 0x1B5A2,
        'α'..='ω' => 0x1D3F9,
        '∂' => 0x1B5C1,
        'ϵ' => 0x1D3CF,
        'ϑ' => 0x1D3F4,
        'ϰ' => 0x1D3D6,
        'ϕ' => 0x1D3F2,
        'ϱ' => 0x1D3D7,
        'ϖ' => 0x1D3F3,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_monospace(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D62F,
        'a'..='z' => 0x1D629,
        '0'..='9' => 0x1D7C6,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_double_struck(ch: char) -> char {
    let delta = match ch {
        'C' => 0x20BF,
        'H' => 0x20C5,
        'N' => 0x20C7,
        'P'..='Q' => 0x20C9,
        'R' => 0x20CB,
        'Z' => 0x20CA,
        'π' => 0x1D7C,
        'γ' => 0x1D8A,
        'Γ' => 0x1DAB,
        'Π' => 0x1D9F,
        '∑' => return '⅀',
        'A'..='Z' => 0x1D4F7,
        'a'..='z' => 0x1D4F1,
        '0'..='9' => 0x1D7A8,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_double_struck_italic(ch: char) -> char {
    let delta = match ch {
        'D' => 0x2101,
        'd'..='e' => 0x20E2,
        'i'..='j' => 0x20DF,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_hebrew(ch: char) -> char {
    let delta = match ch {
        'א'..='ד' => 0x1B65,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}
