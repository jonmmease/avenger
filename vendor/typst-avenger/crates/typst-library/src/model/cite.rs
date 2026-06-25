use crate::diag::{SourceResult, bail};
use crate::engine::Engine;
use crate::foundations::{Content, Label, Packed, StyleChain, Synthesize, cast, elem};
use crate::introspection::Locatable;
use crate::model::CitationForm;
use crate::text::{Lang, Region, TextElem};

#[elem(Locatable, Synthesize)]
pub struct CiteElem {
    #[required]
    pub key: Label,
    pub supplement: Option<Content>,
    #[default(Some(CitationForm::Normal))]
    pub form: Option<CitationForm>,
    #[internal]
    #[synthesized]
    pub lang: Lang,
    #[internal]
    #[synthesized]
    pub region: Option<Region>,
}

impl Synthesize for Packed<CiteElem> {
    fn synthesize(&mut self, _: &mut Engine, styles: StyleChain) -> SourceResult<()> {
        let elem = self.as_mut();
        elem.lang = Some(styles.get(TextElem::lang));
        elem.region = Some(styles.get(TextElem::region));
        Ok(())
    }
}

cast! {
    CiteElem,
    v: Content => v.unpack::<Self>().map_err(|_| "expected citation")?,
}

#[elem(Locatable)]
pub struct CiteGroup {
    #[required]
    pub children: Vec<Content>,
}

impl Packed<CiteGroup> {
    pub fn realize(&self, _: &mut Engine) -> SourceResult<Content> {
        bail!(self.span(), "citations are not available in avenger-typst math fragments")
    }
}
