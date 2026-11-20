-- A histogram of the IMDB ratings of each genre, with the bins of all the movies.
FROM samples.movies
|> WHERE genre IS NOT NULL AND imdb_rating IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(imdb_rating, maxbins => 20), title => 'IMDB rating'),
     y => count(*),
   )
   GROUP BY genre
|> ORDER BY genre
