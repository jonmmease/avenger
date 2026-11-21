-- The movies of each five years, stacked by the four largest genres.
FROM samples.movies
|> WHERE genre IS NOT NULL AND year IS NOT NULL
|> AGGREGATE mark_bar(x => bin(year, step => 5), y => stack(count(*)), color => lump(genre, n => 4))
