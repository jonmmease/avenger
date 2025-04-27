pub mod tree_sitter_ast;
pub mod ast;
pub mod error;

use tree_sitter_ast::SqlExpression;
use tree_sitter::{Query, QueryCursor, StreamingIterator};
use tree_sitter_ast::ts_typed_ast::AstNode;

// mod ast {
//     include!(concat!(env!("OUT_DIR"), "/ast.rs"));
// }


pub fn add(left: u64, right: u64) -> u64 {
    left + right
}
