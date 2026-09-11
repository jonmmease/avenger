use std::{collections::BTreeSet, sync::Arc};

use avenger_text::{
    path::PlainTextPathRun,
    types::{FontStyle, FontWeight, FontWeightNameSpec},
};
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
}

#[derive(Debug, Default)]
pub(crate) struct SvgFontCollector {
    fonts: Vec<SvgFont>,
}

impl SvgFontCollector {
    pub(crate) fn collect_run(
        &mut self,
        run: &PlainTextPathRun,
    ) -> Result<String, AvengerSvgError> {
        let weight = match run.font_weight {
            FontWeight::Name(FontWeightNameSpec::Normal) => 400,
            FontWeight::Name(FontWeightNameSpec::Bold) => 700,
            FontWeight::Number(weight) => weight.round().clamp(1.0, 1000.0) as u16,
        };
        let mut families = Vec::new();
        for resource in &run.font_resources {
            let index = self
                .fonts
                .iter()
                .position(|font| {
                    font.face_index == resource.face_index
                        && font.data == resource.data
                        && font.weight == weight
                        && font.style == run.font_style
                })
                .unwrap_or_else(|| {
                    self.fonts.push(SvgFont {
                        data: resource.data.clone(),
                        face_index: resource.face_index,
                        weight,
                        style: run.font_style,
                        chars: BTreeSet::new(),
                    });
                    self.fonts.len() - 1
                });
            self.fonts[index].chars.extend(run.text.chars());
            families.push(format!("avenger-font-{index}"));
        }
        if families.is_empty() && !run.text.is_empty() {
            return Err(AvengerSvgError::Font(
                "text run has no resolved font resource".into(),
            ));
        }
        Ok(families.join(", "))
    }

    pub(crate) fn font_face_css(&self) -> Result<String, AvengerSvgError> {
        let mut css = String::new();
        for (index, font) in self.fonts.iter().enumerate() {
            let sfnt = standalone_face(&font.data, font.face_index)?;
            // Preserve the complete face when the subsetter cannot retain its tables.
            let (data, mime, format) = match subset_font_to_woff2(&sfnt, &font.chars) {
                Ok(subset) => (subset, "font/woff2", "woff2"),
                Err(_) => (sfnt, "font/otf", "opentype"),
            };
            let style = match font.style {
                FontStyle::Normal => "normal",
                FontStyle::Italic => "italic",
            };
            css.push_str(&format!(
                "@font-face {{\n  font-family: \"avenger-font-{index}\";\n  font-style: {style};\n  font-weight: {};\n  font-display: block;\n  src: url(\"data:{mime};base64,{}\") format(\"{format}\");\n}}\n",
                font.weight, BASE64_STANDARD.encode(data),
            ));
        }
        Ok(css)
    }
}

fn subset_font_to_woff2(data: &[u8], chars: &BTreeSet<char>) -> Result<Vec<u8>, String> {
    let reader = FontReader::new(data).map_err(|err| err.to_string())?;
    let font = reader.read().map_err(|err| err.to_string())?;
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
        let fonts = avenger_text::fonts::registered_default_fonts();
        let data = &fonts[2].data;
        let subset = subset_font_to_woff2(data, &"Axis".chars().collect()).unwrap();
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
    fn extracts_the_requested_collection_face() {
        let fonts = avenger_text::fonts::registered_default_fonts();
        let mut collection = b"ttcf\x00\x01\x00\x00\x00\x00\x00\x02".to_vec();
        collection.resize(20, 0);
        for (i, font) in fonts.iter().take(2).enumerate() {
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
