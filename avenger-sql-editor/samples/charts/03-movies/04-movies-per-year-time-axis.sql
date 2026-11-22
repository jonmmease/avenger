-- The movies of each year, on a time axis.
FROM samples.movies
|> WHERE year IS NOT NULL
|> AGGREGATE mark_area(x => x(date(year, 1, 1), title => 'Year'), y => count(*))
