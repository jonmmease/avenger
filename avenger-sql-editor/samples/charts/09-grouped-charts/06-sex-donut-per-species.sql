-- A donut of the sexes of each species.
FROM samples.penguins
|> AGGREGATE mark_polar_bar(
     theta => stack(count(*)),
     color => sex,
     coord => polar(inner_radius => 0.5),
   )
   GROUP BY species
|> ORDER BY species
