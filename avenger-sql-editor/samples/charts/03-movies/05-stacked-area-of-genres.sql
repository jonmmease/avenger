-- The movies of each year, stacked by the four genres with the most movies and the rest as Other. A
-- stacked area imputes 0 for the years without movies of a genre.
FROM samples.movies
|> WHERE year IS NOT NULL AND genre IS NOT NULL
|> AGGREGATE mark_area(
     x => x(date(year, 1, 1), title => 'Year'),
     y => stack(count(*)),
     color => lump(genre, n => 4),
   )
