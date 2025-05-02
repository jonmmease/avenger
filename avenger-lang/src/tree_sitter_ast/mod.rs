pub mod ts_typed_ast;

include!(concat!(env!("OUT_DIR"), "/ast.rs"));


#[cfg(test)]
mod tests {
    use tree_sitter::{Point, Query, QueryCursor, StreamingIterator};
    use crate::tree_sitter_ast::{self, ts_typed_ast::AstNode};

    use super::*;

    #[test]
    fn it_works() {
        let source = br"
val x: 1 + 2;
Rect {
    x := 10;
}
        ";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_avenger::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let mut cursor = tree.walk();

        let root = tree.root_node();
        
        root.children(&mut cursor).for_each(|child| {
            println!("{:#?}", child);
        });


        // Lookup node at position
        let expr = tree_sitter_ast::File::cast(tree.root_node()).unwrap();
        let statements = expr.statements();
        println!("{:#?}", statements.count());
        for stmt in expr.statements() {
            println!("{:#?}", stmt);
        }

        // Find the smallest node within the root node that spans the given point range
        let point = Point::new(2, 1);
        cursor.goto_first_child_for_point(point);

        let node = cursor.node();
        println!("{:#?}", node);
        let field_name = cursor.field_name();
        println!("{:#?}", field_name);
        let comp_instance = tree_sitter_ast::CompInstance::cast(node.parent().unwrap()).unwrap();
        println!("{:#?}", comp_instance);
        // comp_instance.statements()

        visit_all_sql_expressions(source, &tree);

        // println!("{:#?}", node.utf8_text(source).unwrap());
        // let id = ast::PascalIdentifier::cast(node).unwrap();
        // println!("{:#?}", id);

        
        // let parent = node.parent().unwrap();
        // println!("{:#?}", parent);
        // println!("{:#?}", parent.utf8_text(source).unwrap());

        // parent.field_name_for_child(node)
        // ast::ImportStatement::items();
        // ast::StatementInnerVisitor::new().visit_file(&expr);
    }


    fn visit_all_sql_expressions(source_code: &[u8], tree: &tree_sitter::Tree) {
        // Create a query to match all sql_expression nodes
        let query = Query::new(
            &tree_sitter_avenger::LANGUAGE.into(),
            "(sql_expression) @sql_expr"
        ).expect("Invalid query");
        
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(
            &query, tree.root_node(), source_code
        );
    
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                
                // Now you can work with each SQL expression node
                // You can use the generated Rust types to interact with them:
                if let Some(expr) = SqlExpression::cast(node) {
                    println!("Found SQL expression: {:?}", expr);
                    
                    // If needed, you can use visitors for each individual expression
                    // let result = my_visitor.sql_expression(expr);
                }
            }
        }
    }
}

