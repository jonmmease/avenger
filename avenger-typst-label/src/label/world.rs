//! The world labels are typeset in: their fonts.
//!
//! Mirrors typst-kit's `FontStore` and `FontSlot` (`crates/typst-kit/src/fonts.rs` at v0.15.1).
//! fontdb discovers font files, upstream's `FontInfo::new` describes every face, and a face's
//! `Font` loads on first use. Registered fonts come first, in the given order, then the fonts
//! in the extra directories, then the system's. The book prefers the earliest face among equal
//! matches, so this order is behavior.
//!
//! The book is built on first use rather than when the world is created, so that creating an
//! engine stays as cheap as fontdb's scan.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::typst_library::World;
use crate::typst_library::text::{Font, FontBook, FontInfo, InstanceCache};
use typst_syntax::FileId;

/// The fonts available to labels.
pub struct LabelWorld {
    /// Font data registered by the application, with collection indices.
    registered: Vec<(Arc<[u8]>, u32)>,
    /// The font files fontdb discovered. It also names the generic families.
    db: fontdb::Database,
    /// The fonts' metadata and slots, built on first use.
    fonts: OnceLock<Fonts>,
    /// The instances of the fonts.
    instances: Arc<InstanceCache>,
}

/// The fonts' metadata and the slots they load from, index-aligned.
struct Fonts {
    book: FontBook,
    slots: Vec<FontSlot>,
}

/// Holds a font source and the lazily loaded font itself.
struct FontSlot {
    source: FontSource,
    /// The index in the font collection, or zero for a single font.
    index: u32,
    font: OnceLock<Option<Font>>,
}

/// Where a font's data comes from.
enum FontSource {
    /// Registered data, which fonts share rather than copy.
    Data(Arc<[u8]>),
    /// A file, which is read when the font is first used.
    File(PathBuf),
}

impl LabelWorld {
    /// Creates a world with the `registered` fonts, then the fonts in `font_dirs`, then the
    /// system's fonts if `system_fonts` is set.
    pub fn new(
        registered: impl IntoIterator<Item = (Arc<[u8]>, u32)>,
        font_dirs: &[PathBuf],
        system_fonts: bool,
    ) -> Self {
        let mut db = fontdb::Database::new();
        for dir in font_dirs {
            db.load_fonts_dir(dir);
        }
        if system_fonts {
            db.load_system_fonts();
        }
        Self {
            registered: registered.into_iter().collect(),
            db,
            fonts: OnceLock::new(),
            instances: Arc::new(InstanceCache::default()),
        }
    }

    /// Sets the family that `sans-serif` names.
    pub fn set_sans_serif_family(&mut self, family: &str) {
        self.db.set_sans_serif_family(family);
    }

    /// Sets the family that `monospace` names.
    pub fn set_monospace_family(&mut self, family: &str) {
        self.db.set_monospace_family(family);
    }

    /// The family names in a CSS-style family list, with the generic families `sans-serif`,
    /// `serif`, `monospace`, `cursive` and `fantasy` resolved to the families they name.
    /// Names may be quoted. An empty list names `sans-serif`.
    pub fn families(&self, list: &str) -> Vec<String> {
        let mut families: Vec<String> = list
            .split(',')
            .map(|family| family.trim().trim_matches('"').trim_matches('\''))
            .filter(|family| !family.is_empty())
            .map(|family| self.generic(family).unwrap_or(family).to_string())
            .collect();
        if families.is_empty() {
            families.push(self.db.family_name(&fontdb::Family::SansSerif).to_string());
        }
        families
    }

    /// The family a generic family name names, if it is one.
    fn generic(&self, family: &str) -> Option<&str> {
        let generic: &'static fontdb::Family = match family.to_ascii_lowercase().as_str()
        {
            "sans-serif" | "sans serif" => &fontdb::Family::SansSerif,
            "serif" => &fontdb::Family::Serif,
            "monospace" => &fontdb::Family::Monospace,
            "cursive" => &fontdb::Family::Cursive,
            "fantasy" => &fontdb::Family::Fantasy,
            _ => return None,
        };
        Some(self.db.family_name(generic))
    }

    /// The fonts, described on first use.
    fn fonts(&self) -> &Fonts {
        self.fonts.get_or_init(|| {
            let mut book = FontBook::new();
            let mut slots = Vec::new();
            let mut push = |info, source, index| {
                book.push(info);
                slots.push(FontSlot { source, index, font: OnceLock::new() });
            };

            for (data, index) in &self.registered {
                if let Some(info) = FontInfo::new(data, *index) {
                    push(info, FontSource::Data(data.clone()), *index);
                }
            }

            for face in self.db.faces() {
                let path = match &face.source {
                    fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                        path
                    }
                    // Registered fonts never go into the database.
                    fontdb::Source::Binary(_) => continue,
                };
                let info = self.db.with_face_data(face.id, FontInfo::new).flatten();
                if let Some(info) = info {
                    push(info, FontSource::File(path.clone()), face.index);
                }
            }

            Fonts { book, slots }
        })
    }
}

impl World for LabelWorld {
    fn book(&self) -> &FontBook {
        &self.fonts().book
    }

    fn source(&self, _: FileId) -> Option<&str> {
        None
    }

    fn font(&self, index: usize) -> Option<Font> {
        let slot = self.fonts().slots.get(index)?;
        slot.font
            .get_or_init(|| {
                let data = match &slot.source {
                    FontSource::Data(data) => data.clone(),
                    FontSource::File(path) => read(path)?,
                };
                Font::new_cached(data, slot.index, Arc::downgrade(&self.instances))
            })
            .clone()
    }
}

/// Reads a font file.
fn read(path: &Path) -> Option<Arc<[u8]>> {
    std::fs::read(path).ok().map(Arc::from)
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! A world with the fixture fonts only, so that tests don't depend on the system's fonts.

    use std::io::Read;
    use std::sync::{Arc, LazyLock};

    use super::LabelWorld;
    use crate::label::label_file;
    use crate::typst_library::World;
    use crate::typst_library::text::{Font, FontBook};
    use typst_syntax::FileId;

    /// Lato, DejaVu Sans Mono, Lete Sans Math, Noto Sans Hebrew and Devanagari, and the
    /// `Audit*` fonts, in file-name order as `tools/upstream-typst-probe` loads them: the
    /// book breaks ties between equally good faces by this order.
    const FONTS: &[&[u8]] = &[
        include_bytes!("../../tests/fixtures/fonts/AuditHebrewRegular.ttf.br"),
        include_bytes!("../../tests/fixtures/fonts/AuditNoScriptMetrics.ttf.br"),
        include_bytes!("../../tests/fixtures/fonts/AuditScriptOffsets.ttf.br"),
        avenger_fonts::DEJAVU_SANS_MONO,
        avenger_fonts::LATO_BOLD,
        avenger_fonts::LATO_ITALIC,
        avenger_fonts::LATO_LIGHT,
        include_bytes!("../../tests/fixtures/fonts/Lato-Medium.ttf.br"),
        avenger_fonts::LETE_SANS_MATH_BOLD,
        avenger_fonts::LETE_SANS_MATH,
        include_bytes!("../../tests/fixtures/fonts/NotoSansDevanagari.ttf.br"),
        include_bytes!("../../tests/fixtures/fonts/NotoSansHebrew.ttf.br"),
    ];

    /// A world with the fixture fonts, where `sans-serif` is Lato and `monospace` is DejaVu
    /// Sans Mono.
    pub(crate) fn world() -> LabelWorld {
        let fonts = FONTS.iter().map(|compressed| {
            let mut data = Vec::new();
            brotli::Decompressor::new(*compressed, 4096)
                .read_to_end(&mut data)
                .expect("fixture fonts decompress");
            (Arc::<[u8]>::from(data), 0)
        });
        let mut world = LabelWorld::new(fonts, &[], false);
        world.set_sans_serif_family("Lato");
        world.set_monospace_family("DejaVu Sans Mono");
        world
    }

    /// One [`world`] for all tests that only read it.
    pub(crate) fn shared() -> &'static LabelWorld {
        static WORLD: LazyLock<LabelWorld> = LazyLock::new(world);
        &WORLD
    }

    /// A world that also holds a label's source, so that layout can map glyphs back to it.
    pub(crate) struct WithSource<'a> {
        pub world: &'a LabelWorld,
        pub source: &'a str,
    }

    impl World for WithSource<'_> {
        fn book(&self) -> &FontBook {
            self.world.book()
        }

        fn source(&self, id: FileId) -> Option<&str> {
            (id == label_file()).then_some(self.source)
        }

        fn font(&self, index: usize) -> Option<Font> {
            self.world.font(index)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LabelWorld;
    use super::fixtures::world;
    use crate::typst_library::World;
    use crate::typst_library::layout::Abs;
    use crate::typst_library::text::{
        Font, FontFlags, FontInstance, FontStretch, FontStyle, FontVariant,
        FontVariations, FontWeight,
    };

    fn variant(style: FontStyle, weight: u16) -> FontVariant {
        FontVariant::new(style, FontWeight::from_number(weight), FontStretch::NORMAL)
    }

    /// The family, style and weight of the font at `index`.
    fn describe(world: &LabelWorld, index: Option<usize>) -> String {
        let info = world.book().info(index.expect("a font is selected")).unwrap();
        let variant = info.variant;
        format!("{} {:?} {}", info.family, variant.style, variant.weight.to_number())
    }

    fn select(world: &LabelWorld, family: &str, style: FontStyle, weight: u16) -> String {
        describe(world, world.book().select(family, variant(style, weight)))
    }

    #[test]
    fn registered_fonts_keep_their_order() {
        let world = world();
        let families: Vec<_> =
            world.book().families().map(|(family, _)| family).collect();
        assert_eq!(
            families,
            [
                "AuditHebrewRegular",
                "AuditNoScriptMetrics",
                "AuditScriptOffsets",
                "DejaVu Sans Mono",
                "Lato",
                "Lete Sans Math",
                "Noto Sans Devanagari",
                "Noto Sans Hebrew",
            ]
        );
        // The book lists a family's faces in registration order.
        let lato: Vec<_> = world.book().select_family("lato").collect();
        assert_eq!(lato, [4, 5, 6, 7]);
    }

    #[test]
    fn generic_families_name_the_configured_families() {
        let world = world();
        assert_eq!(world.families("sans-serif"), ["Lato"]);
        assert_eq!(
            world.families(" 'Fira Sans' , monospace"),
            ["Fira Sans", "DejaVu Sans Mono"]
        );
        assert_eq!(world.families(""), ["Lato"]);
        // No option sets the serif family, so it is fontdb's default.
        assert_eq!(world.families("serif"), ["Times New Roman"]);
    }

    #[test]
    fn selection_relaxes_weight_and_style_like_upstream() {
        let world = world();
        assert_eq!(select(&world, "lato", FontStyle::Normal, 700), "Lato Normal 700");
        // Equally distant weights go to the earlier face, here Lato-Bold.
        assert_eq!(select(&world, "lato", FontStyle::Normal, 600), "Lato Normal 700");
        assert_eq!(select(&world, "lato", FontStyle::Normal, 900), "Lato Normal 700");
        assert_eq!(select(&world, "lato", FontStyle::Normal, 100), "Lato Normal 300");
        // Oblique is closer to italic than to normal.
        assert_eq!(select(&world, "lato", FontStyle::Oblique, 400), "Lato Italic 400");
        assert_eq!(
            world.book().select("fira sans", variant(FontStyle::Normal, 400)),
            None
        );
    }

    #[test]
    fn fallback_finds_hebrew_for_emphasized_hebrew() {
        let world = world();
        let book = world.book();
        let italic = variant(FontStyle::Italic, 400);
        let lato = book.info(book.select("lato", italic).unwrap());
        // Both Hebrew faces are equally like Lato, so the shorter family name wins. The
        // variable font's default instance is its thinnest; its weight axis spans 400.
        let hebrew = "Noto Sans Hebrew Normal 100";
        assert_eq!(describe(&world, book.select_fallback(lato, italic, "שלום")), hebrew);
        // Fallback skips leading spaces and default ignorables.
        assert_eq!(
            describe(&world, book.select_fallback(lato, italic, " \u{200D}ש")),
            hebrew
        );
        assert_eq!(book.select_fallback(lato, italic, " \u{200D}"), None);
    }

    #[test]
    fn fonts_and_instances_load_once() {
        let world = world();
        let font = world.font(0).unwrap();
        assert_eq!(font, world.font(0).unwrap());
        assert_ne!(font, world.font(1).unwrap());
        assert!(world.font(world.book().families().count() + 100).is_none());

        let size = Abs::pt(12.0);
        let regular = variant(FontStyle::Normal, 400);
        let a = font.clone().instantiate(regular, size, &FontVariations::default());
        let b = font.clone().instantiate(regular, size, &FontVariations::default());
        assert!(std::ptr::eq(a.metrics(), b.metrics()));

        // A font loaded outside a world has no cache.
        let loose = Font::new(font.data().clone(), 0).unwrap();
        assert_ne!(loose, font);
        let c = loose.clone().instantiate(regular, size, &FontVariations::default());
        let d = loose.instantiate(regular, size, &FontVariations::default());
        assert!(!std::ptr::eq(c.metrics(), d.metrics()));
    }

    #[test]
    fn variable_fonts_instantiate_their_axes() {
        let world = world();
        let book = world.book();
        let hebrew = book
            .select("noto sans hebrew", variant(FontStyle::Normal, 400))
            .unwrap();
        let font = world.font(hebrew).unwrap();
        assert!(font.info().flags.contains(FontFlags::VARIABLE));
        let bold = font.instantiate(
            variant(FontStyle::Normal, 700),
            Abs::pt(12.0),
            &FontVariations::default(),
        );
        let wght = bold.variations().0.iter().find(|(tag, _)| tag.to_bytes() == *b"wght");
        assert_eq!(wght.map(|(_, value)| value.0), Some(700.0));
    }

    #[test]
    fn math_fonts_have_math_constants() {
        let world = world();
        let book = world.book();
        let math = book
            .select("lete sans math", variant(FontStyle::Normal, 400))
            .unwrap();
        assert!(book.info(math).unwrap().flags.contains(FontFlags::MATH));
        let mono = book
            .select("dejavu sans mono", variant(FontStyle::Normal, 400))
            .unwrap();
        assert!(book.info(mono).unwrap().flags.contains(FontFlags::MONOSPACE));

        let instance = world.font(math).unwrap().instantiate(
            variant(FontStyle::Normal, 400),
            Abs::pt(12.0),
            &FontVariations::default(),
        );
        let constants = instance.math();
        assert!(constants.axis_height.get() > 0.0);
        assert!(std::ptr::eq(constants, instance.math()));
        assert_eq!(constants.script_percent_scale_down, 70);
    }

    #[test]
    fn worlds_fonts_and_instances_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LabelWorld>();
        assert_send_sync::<Font>();
        assert_send_sync::<FontInstance>();
    }
}
