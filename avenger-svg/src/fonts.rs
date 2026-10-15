use std::{collections::BTreeSet, sync::Arc};

use avenger_typst_label::{FontStyle, TextRun};
use base64::{prelude::BASE64_STANDARD, Engine};
use font_subset::FontReader;

use crate::error::AvengerSvgError;

#[derive(Debug)]
struct SvgFont {
    data: Arc<[u8]>,
    face_index: u32,
    weight: u16,
    style: FontStyle,
    chars: BTreeSet<char>,
    embedding: Embedding,
    /// The family name of the embedded face, or `None` when its license forbids embedding.
    family: Option<String>,
}

/// How a face's license, the embedding flags of its `OS/2` table, lets a document embed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Embedding {
    /// Embed the characters the text uses, or the whole face when subsetting would drop its
    /// tables.
    Subset,
    /// Embed the whole face.
    Whole,
    /// Don't embed the face.
    Forbidden,
}

impl Embedding {
    /// A restricted license, or one that allows embedding only bitmaps, forbids embedding. A
    /// face without an `OS/2` table records no restriction.
    fn of(data: &[u8], index: u32) -> Result<Self, AvengerSvgError> {
        let face = ttf_parser::Face::parse(data, index)
            .map_err(|err| AvengerSvgError::Font(format!("invalid resolved font face: {err}")))?;
        let Some(os2) = face.tables().os2 else {
            return Ok(Self::Subset);
        };
        Ok(
            if os2.permissions() == Some(ttf_parser::Permissions::Restricted)
                || !os2.is_outline_embedding_allowed()
            {
                Self::Forbidden
            } else if os2.is_subsetting_allowed() {
                Self::Subset
            } else {
                Self::Whole
            },
        )
    }
}

#[derive(Debug, Default)]
pub(crate) struct SvgFontCollector {
    fonts: Vec<SvgFont>,
    embedded: usize,
}

impl SvgFontCollector {
    /// The family name that draws the run: its embedded face's, or the face's own family when
    /// its license forbids embedding.
    pub(crate) fn collect_run(&mut self, run: &TextRun) -> Result<String, AvengerSvgError> {
        let weight = run.weight.to_number();
        let data = run.font.data();
        let face_index = run.font.index();
        let index = match self.fonts.iter().position(|font| {
            font.face_index == face_index
                && (Arc::ptr_eq(&font.data, data) || font.data == *data)
                && font.weight == weight
                && font.style == run.style
        }) {
            Some(index) => index,
            None => {
                let embedding = Embedding::of(data, face_index)?;
                let family = (embedding != Embedding::Forbidden).then(|| {
                    self.embedded += 1;
                    format!("avenger-font-{}", self.embedded - 1)
                });
                self.fonts.push(SvgFont {
                    data: data.clone(),
                    face_index,
                    weight,
                    style: run.style,
                    chars: BTreeSet::new(),
                    embedding,
                    family,
                });
                self.fonts.len() - 1
            }
        };
        let font = &mut self.fonts[index];
        Ok(match &font.family {
            Some(family) => {
                font.chars.extend(run.text.chars());
                family.clone()
            }
            None => run.font.family().to_string(),
        })
    }

    pub(crate) fn font_face_css(&self) -> Result<String, AvengerSvgError> {
        let mut css = String::new();
        for font in &self.fonts {
            let Some(family) = &font.family else {
                continue;
            };
            let sfnt = standalone_face(&font.data, font.face_index)?;
            let chars = (font.embedding == Embedding::Subset).then_some(&font.chars);
            // Preserve the complete face when the subsetter cannot read it.
            let (data, mime, format) = match face_to_woff2(&sfnt, chars) {
                Ok(woff2) => (woff2, "font/woff2", "woff2"),
                Err(_) => (sfnt, "font/otf", "opentype"),
            };
            let style = crate::renderer::font_style_value(font.style);
            css.push_str(&format!(
                "@font-face {{\n  font-family: \"{family}\";\n  font-style: {style};\n  font-weight: {};\n  font-display: block;\n  src: url(\"data:{mime};base64,{}\") format(\"{format}\");\n}}\n",
                font.weight, BASE64_STANDARD.encode(data),
            ));
        }
        Ok(css)
    }
}

/// The face as WOFF2, subset to `chars` when they're given and subsetting keeps its tables.
fn face_to_woff2(data: &[u8], chars: Option<&BTreeSet<char>>) -> Result<Vec<u8>, String> {
    let reader = FontReader::new(data).map_err(|err| err.to_string())?;
    let font = reader.read().map_err(|err| err.to_string())?;
    let Some(chars) = chars else {
        return Ok(font.to_woff2());
    };
    // Character subsetting drops shaping and color tables. Keep those faces intact.
    let protected = [
        *b"GSUB", *b"GPOS", *b"kern", *b"morx", *b"mort", *b"COLR", *b"CBDT", *b"sbix", *b"SVG ",
        *b"fvar",
    ];
    if reader.raw_tables().any(|(tag, _)| {
        protected
            .iter()
            .any(|protected| tag.to_string().as_bytes() == protected)
    }) {
        return Ok(font.to_woff2());
    }
    let chars: BTreeSet<_> = chars
        .iter()
        .copied()
        .filter(|ch| font.contains_char(*ch))
        .collect();
    if chars.is_empty() {
        return Err("font contains no requested characters".into());
    }
    let subset = font.subset(&chars).map_err(|err| err.to_string())?;
    subset
        .validate()
        .map_err(|err| err.to_string())?
        .into_result()
        .map_err(|err| err.to_string())?;
    Ok(subset.to_woff2())
}

/// Extract one collection face as an SFNT with absolute table offsets and a new checksum.
fn standalone_face(data: &[u8], index: u32) -> Result<Vec<u8>, AvengerSvgError> {
    let face = ttf_parser::Face::parse(data, index)
        .map_err(|err| AvengerSvgError::Font(format!("invalid resolved font face: {err}")))?;
    if !data.starts_with(b"ttcf") {
        return Ok(data.to_vec());
    }
    let raw = face.raw_face();
    let mut records = raw.table_records.into_iter().collect::<Vec<_>>();
    records.sort_by_key(|record| record.tag);
    let count = records.len() as u16;
    let mut output = vec![0u8; 12 + records.len() * 16];
    let signature = if raw.table(ttf_parser::Tag::from_bytes(b"CFF ")).is_some()
        || raw.table(ttf_parser::Tag::from_bytes(b"CFF2")).is_some()
    {
        *b"OTTO"
    } else {
        [0, 1, 0, 0]
    };
    output[..4].copy_from_slice(&signature);
    output[4..6].copy_from_slice(&count.to_be_bytes());
    let selector = count.ilog2() as u16;
    let search_range = (1u16 << selector) * 16;
    output[6..8].copy_from_slice(&search_range.to_be_bytes());
    output[8..10].copy_from_slice(&selector.to_be_bytes());
    output[10..12].copy_from_slice(&(count * 16 - search_range).to_be_bytes());
    let mut head_offset = None;
    for (i, record) in records.iter().enumerate() {
        let table = raw
            .table(record.tag)
            .ok_or_else(|| AvengerSvgError::Font("invalid font table range".into()))?;
        let offset = output.len();
        output.extend_from_slice(table);
        output.resize(output.len().next_multiple_of(4), 0);
        if record.tag == ttf_parser::Tag::from_bytes(b"head") {
            output[offset + 8..offset + 12].fill(0);
            head_offset = Some(offset);
        }
        let checksum = checksum(&output[offset..]);
        let record_offset = 12 + i * 16;
        output[record_offset..record_offset + 4].copy_from_slice(&record.tag.to_bytes());
        output[record_offset + 4..record_offset + 8].copy_from_slice(&checksum.to_be_bytes());
        output[record_offset + 8..record_offset + 12]
            .copy_from_slice(&(offset as u32).to_be_bytes());
        output[record_offset + 12..record_offset + 16]
            .copy_from_slice(&(table.len() as u32).to_be_bytes());
    }
    if let Some(offset) = head_offset {
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&output));
        output[offset + 8..offset + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    Ok(output)
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks_exact(4).fold(0u32, |sum, bytes| {
        sum.wrapping_add(u32::from_be_bytes(bytes.try_into().unwrap()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_shaping_tables_in_embedded_fonts() {
        let fonts = avenger_typst_label::bundled_font_options().registered_fonts;
        let data = &fonts[2].data;
        let subset = face_to_woff2(data, Some(&"Axis".chars().collect())).unwrap();
        assert!(subset.len() < data.len());
        let reader = FontReader::new(&subset).unwrap();
        let font = reader.read().unwrap();
        assert!(font.contains_char('A'));
        assert!(font.contains_char('Z'));
        let original = FontReader::new(data).unwrap();
        let original_gsub = original
            .raw_tables()
            .find(|(tag, _)| tag.to_string() == "GSUB")
            .unwrap()
            .1;
        let embedded_gsub = reader
            .raw_tables()
            .find(|(tag, _)| tag.to_string() == "GSUB")
            .unwrap()
            .1;
        assert_eq!(original_gsub, embedded_gsub);
    }

    #[test]
    fn license_flags_decide_how_a_face_embeds() {
        use avenger_render_fixtures::scene::with_embedding_flags;
        let fonts = avenger_typst_label::bundled_font_options().registered_fonts;
        for (fs_type, expected) in [
            (0x0000, Embedding::Subset),    // Installable
            (0x0002, Embedding::Forbidden), // Restricted
            (0x0004, Embedding::Subset),    // Preview and print
            (0x0008, Embedding::Subset),    // Editable
            (0x0104, Embedding::Whole),     // Preview and print, without subsetting
            (0x0200, Embedding::Forbidden), // Bitmaps only
        ] {
            let data = with_embedding_flags(&fonts[0].data, fs_type);
            assert_eq!(Embedding::of(&data, 0).unwrap(), expected, "{fs_type:#06x}");
        }
    }

    #[test]
    fn extracts_the_requested_collection_face() {
        let fonts = avenger_typst_label::bundled_font_options().registered_fonts;
        let italic = |font: &&avenger_typst_label::RegisteredFont| {
            ttf_parser::Face::parse(&font.data, 0).unwrap().is_italic()
        };
        let faces = [
            fonts.iter().find(|font| !italic(font)).unwrap(),
            fonts.iter().find(italic).unwrap(),
        ];
        let mut collection = b"ttcf\x00\x01\x00\x00\x00\x00\x00\x02".to_vec();
        collection.resize(20, 0);
        for (i, font) in faces.into_iter().enumerate() {
            let offset = collection.len();
            collection[12 + i * 4..16 + i * 4].copy_from_slice(&(offset as u32).to_be_bytes());
            let mut data = font.data.to_vec();
            let count = u16::from_be_bytes(data[4..6].try_into().unwrap()) as usize;
            for record in 0..count {
                let position = 12 + record * 16 + 8;
                let table_offset =
                    u32::from_be_bytes(data[position..position + 4].try_into().unwrap());
                data[position..position + 4]
                    .copy_from_slice(&(table_offset + offset as u32).to_be_bytes());
            }
            collection.extend(data);
            collection.resize(collection.len().next_multiple_of(4), 0);
        }
        for i in 0..2 {
            let sfnt = standalone_face(&collection, i).unwrap();
            let face = ttf_parser::Face::parse(&sfnt, 0).unwrap();
            assert_eq!(face.is_italic(), i == 1);
            assert_eq!(checksum(&sfnt), 0xB1B0_AFBA);
        }
    }
}
