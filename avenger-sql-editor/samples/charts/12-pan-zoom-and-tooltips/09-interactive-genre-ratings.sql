-- The ratings of the movies, with the four largest genres and the rest as Other, with pan, zoom,
-- and a tooltip.
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_symbol(
     x => rotten_tomatoes_rating,
     y => imdb_rating,
     color => lump(genre, n => 4),
     opacity => 0.4,
     tools => [navigate(), tooltip(nearest => TRUE, max_radius => 10)],
   )
