-- The penguins of each species, stacked by whether they reach the body mass in the variable
-- `heavy`, which the title states too.
DECLARE heavy INT64 DEFAULT 4500;
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> EXTEND if(body_mass_g >= heavy, 'heavy', 'light') AS class
|> AGGREGATE mark_bar(
     x => species,
     y => stack(count(*)),
     color => class,
     title => 'Penguins of at least ' || CAST(heavy AS STRING) || ' g',
   )
