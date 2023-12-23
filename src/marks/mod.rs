use crate::marks::symbol::SymbolMarkRenderer;
use crate::marks::rect::RectMarkRenderer;
pub mod symbol;
pub mod vertex;
pub mod rect;

pub enum MarkRenderer {
    Symbol(SymbolMarkRenderer),
    Rect(RectMarkRenderer)
}
