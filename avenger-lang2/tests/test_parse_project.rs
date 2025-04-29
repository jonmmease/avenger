use avenger_lang2::parser::parse_project;
use std::path::PathBuf;

#[test]
fn test_parse_project() {
    let project_path = PathBuf::from("tests/projects/project1");
    
    let project = parse_project(&project_path).unwrap();
    
    // Print the files that were found
    println!("Parsed project with {} files:", project.files.len());
    for file in &project.files {
        println!("  - {} (path: '{}')", file.name, file.path);
    }
    
    // Assert we found at least two files
    assert_eq!(project.files.len(), 3, "Expected to find 2 files in the project directory");
}

