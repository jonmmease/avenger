-- The penguins of each species as a donut.
FROM samples.penguins
|> AGGREGATE mark_polar_bar(
     theta => stack(count(*)),
     color => species,
     coord => polar(inner_radius => 0.5),
   )
