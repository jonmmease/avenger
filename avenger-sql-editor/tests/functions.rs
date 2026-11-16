//! The functions that the data sidebar lists.

use avenger_sql_editor::query::{functions, Function};

#[test]
fn the_sidebar_lists_the_families_of_avenger_sql() {
    let found = functions();
    let names = |list: &[Function]| -> Vec<String> {
        list.iter().map(|function| function.name.clone()).collect()
    };
    assert!(names(&found.marks).contains(&"bar".to_string()));
    assert!(names(&found.compositions).contains(&"facet_col".to_string()));
    assert!(names(&found.constructors).contains(&"scaled".to_string()));
    assert_eq!(
        names(&found.transforms),
        ["bin", "impute", "lump", "stack", "timeunit"]
    );
}
