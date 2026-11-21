-- The ratings of the movies, colored by as many of the largest genres as the variable `genres`
-- names, and the rest as Other.
DECLARE genres INT64 DEFAULT 5;
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_symbol(
     x => rotten_tomatoes_rating,
     y => imdb_rating,
     color => lump(genre, n => genres),
     opacity => 0.4,
   )
