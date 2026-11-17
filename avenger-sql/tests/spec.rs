//! The examples of `SPEC.md`, the short guide to the language, which agents copy.

#[test]
fn the_examples_of_the_guide_parse_without_errors() {
    let guide = include_str!("../SPEC.md");
    let examples: Vec<&str> = (guide.split("```sql\n").skip(1))
        .map(|rest| rest.split("```").next().unwrap())
        .collect();
    assert_eq!(examples.len(), 7);
    for example in examples {
        let parse = avenger_sql_syntax::parse(example);
        assert!(
            parse.diagnostics().is_empty(),
            "{example}\n{:?}",
            parse.diagnostics()
        );
    }
}
