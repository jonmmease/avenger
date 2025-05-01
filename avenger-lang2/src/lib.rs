pub mod tree_sitter_ast;
pub mod ast;
pub mod visitor;
pub mod error;
pub mod parser;
pub mod loader;


pub fn add(left: u64, right: u64) -> u64 {
    left + right
}
