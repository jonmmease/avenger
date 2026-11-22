-- A histogram of the IMDB ratings of each genre, in rows of at most four.
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_bar(x => bin(imdb_rating), y => count(*)) AS chart GROUP BY genre
|> AGGREGATE facet_wrap(chart, genre, max_columns => 4) AS chart
