-- A histogram with a title, a navigation, and tooltips.
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => bin(body_mass_g, maxbins => 20),
     y => stack(count(*)),
     color => species,
     title => 'Body mass of the penguins',
     tools => [navigate(), tooltip()],
   )
