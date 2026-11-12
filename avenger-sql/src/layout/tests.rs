//! The layout of the SQL that `lift` writes.

use datafusion::sql::sqlparser::{
    dialect::GenericDialect,
    tokenizer::{Token, Tokenizer, Whitespace},
};

use super::lay_out;

/// The tokens of `sql`, without the whitespace between them.
fn tokens(sql: &str) -> Vec<Token> {
    (Tokenizer::new(&GenericDialect {}, sql).tokenize().unwrap())
        .into_iter()
        .filter(|token| {
            !matches!(
                token,
                Token::Whitespace(Whitespace::Space | Whitespace::Tab | Whitespace::Newline)
            )
        })
        .collect()
}

const PIPE: &str = "FROM penguins |> WHERE body_mass_g > 3000 |> AGGREGATE count(*) AS n, symbol(x => scaled(flipper_length_mm, scale => interval_fit(zero => false)), y => body_mass_g, color => sex) AS chart GROUP BY species, island |> ORDER BY species";

const JOIN: &str = "|> AGGREGATE count(*) AS n GROUP BY island |> AS aggregates |> LEFT OUTER JOIN (FROM penguins |> WHERE body_mass_g IS NOT NULL |> AGGREGATE count(*) AS n GROUP BY island, species |> AGGREGATE bar(x => species, y => n) AS bars GROUP BY island) AS charts ON aggregates.island IS NOT DISTINCT FROM charts.island |> SELECT aggregates.island, aggregates.n, charts.bars";

/// Each pipe operator starts a line, and a long list and the long arguments of a call take one
/// line each.
#[test]
fn a_pipe_query_takes_a_line_for_each_operator() {
    let expected = "\
FROM penguins
|> WHERE body_mass_g > 3000
|> AGGREGATE
     count(*) AS n,
     symbol(
       x => scaled(flipper_length_mm, scale => interval_fit(zero => false)),
       y => body_mass_g,
       color => sex) AS chart
   GROUP BY species, island
|> ORDER BY species";
    assert_eq!(lay_out(PIPE), expected);
}

/// The query of a join target takes lines of its own inside its parentheses, and the `FROM` of
/// `IS NOT DISTINCT FROM` starts no line.
#[test]
fn the_query_of_a_join_target_is_indented() {
    let expected = "\
|> AGGREGATE count(*) AS n
   GROUP BY island
|> AS aggregates
|> LEFT OUTER JOIN (
     FROM penguins
     |> WHERE body_mass_g IS NOT NULL
     |> AGGREGATE count(*) AS n
        GROUP BY island, species
     |> AGGREGATE bar(x => species, y => n) AS bars
        GROUP BY island
   ) AS charts ON aggregates.island IS NOT DISTINCT FROM charts.island
|> SELECT aggregates.island, aggregates.n, charts.bars";
    assert_eq!(lay_out(JOIN), expected);
}

/// The layout changes only the whitespace between tokens.
#[test]
fn the_layout_keeps_every_token() {
    for sql in [
        PIPE,
        JOIN,
        "FROM t |> SELECT 'a  |>  b' AS s |> WHERE x IN (1, 2)",
    ] {
        assert_eq!(tokens(&lay_out(sql)), tokens(sql), "{sql}");
    }
}
