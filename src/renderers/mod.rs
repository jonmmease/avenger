use crate::renderers::rect::RectMarkRenderer;
use crate::renderers::symbol::SymbolMarkRenderer;
pub mod canvas;
pub mod rect;
pub mod symbol;
pub mod vertex;

pub enum MarkRenderer {
    Symbol(SymbolMarkRenderer),
    Rect(RectMarkRenderer),
}
