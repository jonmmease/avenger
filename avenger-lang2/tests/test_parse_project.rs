use avenger_lang2::{parser::parse_project, visitor::{AvengerVisitor, VisitorContext}};
use sqlparser::ast::Visitor;
use std::{ops::ControlFlow, path::PathBuf};

#[test]
fn test_parse_project() {
    let project_path = PathBuf::from("tests/projects/project1");
    
    let project = parse_project(&project_path).unwrap();
    
    // Print the files that were found
    println!("Parsed project with {} files:", project.files.len());
    for file in project.files.values() {
        println!("  - {} (path: '{}')", file.name, file.path);
    }
    
    // Assert we found at least two files
    assert_eq!(project.files.len(), 3, "Expected to find 2 files in the project directory");

    // Visit the project with the BindingVisitor
    let mut visitor = BindingVisitor {};
    project.visit(&mut visitor);
}


pub struct BindingVisitor {}

impl Visitor for BindingVisitor {
    type Break = ();
}

impl AvengerVisitor for BindingVisitor {
    fn post_visit_prop_binding(&mut self, _statement: &avenger_lang2::ast::PropBinding, _context: &VisitorContext) -> std::ops::ControlFlow<Self::Break> {
        println!("Visited prop binding at '{}' {:?}", _context.path.join("."), _statement);
        ControlFlow::Continue(())
    }
}