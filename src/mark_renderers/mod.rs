use crate::mark_renderers::symbol::SymbolMarkRenderer;
use crate::mark_renderers::rect::RectMarkRenderer;
pub mod symbol;
pub mod vertex;
pub mod rect;

pub enum MarkRenderer {
    Symbol(SymbolMarkRenderer),
    Rect(RectMarkRenderer)
}
