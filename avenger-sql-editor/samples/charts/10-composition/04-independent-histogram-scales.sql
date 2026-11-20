-- The same histograms, with the bins of each genre and a y scale for each panel.
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_bar(x => bin(imdb_rating) OVER panel, y => count(*)) AS chart GROUP BY genre
|> AGGREGATE facet_wrap(
     chart,
     genre,
     max_columns => 4,
     resolve => resolve(y => 'independent'),
   ) AS chart
