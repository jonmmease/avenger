-- A histogram of body mass whose bins are as wide as the variable `bin_width`, which the title
-- states too.
DECLARE bin_width FLOAT64 DEFAULT 250;
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(body_mass_g, step => bin_width), title => 'Body mass (g)'),
     y => stack(count(*)),
     color => species,
     title => 'Body mass in bins of ' || CAST(bin_width AS STRING) || ' g',
   )
