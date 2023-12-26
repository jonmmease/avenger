use crate::renderers::symbol::SymbolMarkRenderer;
use crate::renderers::rect::RectMarkRenderer;
pub mod symbol;
pub mod vertex;
pub mod rect;
pub mod canvas;

pub enum MarkRenderer {
    Symbol(SymbolMarkRenderer),
    Rect(RectMarkRenderer)
}
