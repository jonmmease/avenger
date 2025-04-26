use std::marker::PhantomData;
use std::num::NonZeroU16;

use tree_sitter::{Node, TreeCursor};

// use crate::generate::generate;

pub trait AstNode<'tree>: Sized {
    fn can_cast(kind: u16) -> bool;

    fn cast(node: Node<'tree>) -> Option<Self>;

    fn node(&self) -> Node<'tree>;

    fn utf8_text<'a>(&self, source: &'a [u8]) -> Result<&'a str, std::str::Utf8Error> {
        self.node().utf8_text(source)
    }
}

pub struct MissingNodeChildError<'tree> {
    pub node: tree_sitter::Node<'tree>,
    pub field_id: u16,
}

impl<'tree> MissingNodeChildError<'tree> {
    pub fn new(node: tree_sitter::Node<'tree>, field_id: u16) -> Self {
        Self { node, field_id }
    }
}

impl<'tree> std::fmt::Debug for MissingNodeChildError<'tree> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissingNodeChildError")
            .field("node", &self.node.id() as _)
            .field("field_id", &self.field_id as _)
            .finish()
    }
}

impl<'tree> std::fmt::Display for MissingNodeChildError<'tree> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let field_name = self.node.language().field_name_for_id(self.field_id);
        write!(f, "missing child node for")?;
        if let Some(field_name) = field_name {
            write!(f, " field '{field_name}'")?;
        } else {
            write!(f, " unknown field")?;
        }
        let position = self.node.start_position();
        write!(f, " at {}:{}", position.row, position.column)
    }
}

impl<'tree> std::error::Error for MissingNodeChildError<'tree> {}

#[doc(hidden)]
pub enum Children<'tree, T: AstNode<'tree>> {
    Empty,
    Walking {
        cursor: TreeCursor<'tree>,
        field_id: NonZeroU16,
        _marker: PhantomData<T>,
    },
}

impl<'tree, T: AstNode<'tree>> Children<'tree, T> {
    #[doc(hidden)]
    pub fn new(node: tree_sitter::Node<'tree>, field_id: NonZeroU16) -> Self {
        // TODO probably faster to hardcode the ID in the build phase.
        let mut cursor = node.walk();
        cursor.goto_first_child();
        Self::Walking {
            cursor,
            field_id,
            _marker: Default::default(),
        }
    }
}

impl<'tree, T: AstNode<'tree>> Iterator for Children<'tree, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match std::mem::replace(self, Children::Empty) {
            Children::Empty => None,
            Children::Walking {
                mut cursor,
                field_id,
                _marker,
            } => loop {
                let result = if cursor.field_id() == Some(field_id) {
                    T::cast(cursor.node())
                } else {
                    None
                };
                if !cursor.goto_next_sibling() {
                    return result;
                }
                if result.is_some() {
                    _ = std::mem::replace(
                        self,
                        Children::Walking {
                            cursor,
                            field_id,
                            _marker,
                        },
                    );
                    return result;
                }
            },
        }
    }
}
