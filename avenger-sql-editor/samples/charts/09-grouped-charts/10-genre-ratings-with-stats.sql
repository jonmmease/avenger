-- The ratings of the movies of each genre, with the number and the mean IMDB rating of its movies,
-- from the genre with the most movies.
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE
     count(*) AS movies,
     round(avg(imdb_rating), 1) AS mean_imdb_rating,
     mark_symbol(
       x => x(rotten_tomatoes_rating, scale => scale(domain => [0, 100])),
       y => y(imdb_rating, scale => scale(domain => [0, 10])),
       opacity => 0.4,
     ) AS chart,
   GROUP BY genre
|> ORDER BY movies DESC
