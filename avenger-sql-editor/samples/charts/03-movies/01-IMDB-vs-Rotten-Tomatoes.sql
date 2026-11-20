-- The IMDB rating against the Rotten Tomatoes rating.
FROM samples.movies
|> AGGREGATE mark_symbol(x => rotten_tomatoes_rating, y => imdb_rating, opacity => 0.3, size => 20)
