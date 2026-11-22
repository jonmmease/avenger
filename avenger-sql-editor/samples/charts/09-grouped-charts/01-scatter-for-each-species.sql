-- Body mass against flipper length for each species.
FROM samples.penguins
|> AGGREGATE mark_symbol(
     x => x(flipper_length_mm, scale => scale(zero => FALSE)),
     y => y(body_mass_g, scale => scale(zero => FALSE)),
     color => sex,
   )
   GROUP BY species
|> ORDER BY species
