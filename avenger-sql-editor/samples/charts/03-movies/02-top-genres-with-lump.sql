-- The movies of each genre, with the largest genres, as many as the variable `top`, and the rest as
-- Other, from `lump`.
DECLARE top INT64 DEFAULT 5;
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_bar(x => count(*), y => lump(genre, n => top) ORDER BY count(*) DESC)
