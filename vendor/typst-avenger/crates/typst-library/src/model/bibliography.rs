use crate::diag::{SourceResult, bail};
use crate::engine::Engine;
use crate::foundations::{Cast, Content, Derived, elem};
use crate::introspection::{Locatable, Location};
use crate::layout::Length;
use typst_syntax::Span;

#[elem(Locatable)]
pub struct BibliographyElem {
    #[required]
    pub path: Content,
}

impl BibliographyElem {
    pub fn has(_: &mut Engine, _: crate::foundations::Label, _: Span) -> bool {
        false
    }
}

impl crate::foundations::Packed<BibliographyElem> {
    pub fn realize_title(&self, _: crate::foundations::StyleChain) -> Option<Content> {
        None
    }
}

#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum CitationForm {
    #[default]
    Normal,
    Prose,
    Full,
    Author,
    Year,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct CslSource;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct CslStyle;

pub struct Bibliography {
    pub entries: Vec<BibliographyEntry>,
    pub hanging_indent: bool,
}

pub struct BibliographyEntry {
    pub prefix: Option<Content>,
    pub body: Content,
    pub backlink: Location,
}

pub struct Works;

impl Works {
    pub fn generate(_: &mut Engine, _: Span) -> SourceResult<std::sync::Arc<Works>> {
        Ok(std::sync::Arc::new(Works))
    }

    pub fn bibliography(
        &self,
        _location: Location,
        span: Span,
    ) -> SourceResult<Bibliography> {
        bail!(span, "bibliographies are not available in avenger-typst math fragments")
    }

    pub fn citation(&self, _location: Location, span: Span) -> SourceResult<Content> {
        bail!(span, "citations are not available in avenger-typst math fragments")
    }
}

#[elem]
pub struct CslLightElem {
    #[required]
    pub body: Content,
}

#[elem]
pub struct CslIndentElem {
    #[required]
    pub body: Content,
    pub amount: Length,
}

impl CslStyle {
    pub fn load(
        _: &mut Engine,
        _: typst_syntax::Spanned<CslSource>,
    ) -> SourceResult<Derived<CslSource, CslStyle>> {
        bail!(typst_syntax::Span::detached(), "bibliographies are not available in avenger-typst math fragments")
    }
}
