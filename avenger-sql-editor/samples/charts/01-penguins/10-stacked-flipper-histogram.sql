-- A histogram of flipper length, stacked by species, in at most as many bins as the variable
-- `bins`.
DECLARE bins FLOAT64 DEFAULT 20;
FROM samples.penguins
|> WHERE flipper_length_mm IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(flipper_length_mm, maxbins => bins), title => 'Flipper length (mm)'),
     y => y(stack(count(*)), title => 'Penguins'),
     color => species,
   )
