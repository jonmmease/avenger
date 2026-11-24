//! The layout of text in lines of at most 100 characters: groups that stay on one line when they
//! fit, lists that break with one item to a line, and lines that always break.

use avenger_sql_syntax::layout::{list, Doc, INDENT, WIDTH};

fn text(text: &str) -> Doc {
    Doc::text(text)
}

fn call(name: &str, items: Vec<Doc>) -> Doc {
    list(format!("{name}("), items, vec![], ")", INDENT)
}

#[test]
fn a_list_that_fits_stays_on_one_line_without_a_comma_before_its_closer() {
    let doc = call(
        "mark_bar",
        vec![text("x => species"), text("y => count(*)")],
    );
    assert_eq!(doc.render(0), "mark_bar(x => species, y => count(*))");
}

#[test]
fn a_list_that_does_not_fit_breaks_with_one_item_to_a_line_and_a_comma_after_each() {
    let long = "a".repeat(WIDTH);
    let doc = call("f", vec![text(&long), text("b")]);
    assert_eq!(doc.render(0), format!("f(\n    {long},\n    b,\n)"));
}

#[test]
fn an_inner_list_decides_again_at_the_depth_of_its_line() {
    let inner = call("y", vec![text("count(*)"), text("title => 'N'")]);
    let long = format!("title => '{}'", "t".repeat(90));
    let doc = call(
        "mark_bar",
        vec![
            Doc::concat([text("y => "), inner]),
            Doc::concat([text("title => "), call("title", vec![text(&long)])]),
        ],
    );
    assert_eq!(
        doc.render(0),
        format!(
            "mark_bar(\n    y => y(count(*), title => 'N'),\n    title => title(\n        {long},\n    ),\n)"
        )
    );
}

#[test]
fn the_text_after_a_list_counts_toward_its_line() {
    // 98 characters fit, and the text after the closer makes the line too long.
    let doc = call("f", vec![text(&"a".repeat(95))]);
    assert_eq!(doc.render(0).lines().count(), 1);
    let after = Doc::concat([doc, text(" AS chart")]);
    assert_eq!(after.render(0).lines().count(), 3);
}

#[test]
fn a_broken_list_and_a_line_that_always_breaks_break_their_groups() {
    let doc = call("f", vec![text("a")]).broken();
    assert_eq!(doc.render(0), "f(\n    a,\n)");
    let doc = call(
        "f",
        vec![Doc::concat([text("-- why"), Doc::HardLine, text("a")])],
    );
    assert_eq!(doc.render(0), "f(\n    -- why\n    a,\n)");
}

#[test]
fn a_blank_line_has_no_indentation() {
    let doc = Doc::nest(
        2,
        Doc::concat([text("a"), Doc::HardLine, Doc::HardLine, text("b")]),
    );
    assert_eq!(doc.render(0), "a\n\n  b");
}

#[test]
fn a_nest_indents_from_the_column_where_the_layout_starts() {
    let doc = Doc::nest(
        5,
        Doc::group(Doc::concat([
            text("|> AGGREGATE "),
            call("f", vec![text(&"a".repeat(90))]),
            Doc::Line,
            text("GROUP BY k"),
        ])),
    );
    assert_eq!(
        doc.render(2),
        format!(
            "|> AGGREGATE f(\n         {},\n     )\n     GROUP BY k",
            "a".repeat(90)
        )
    );
}
