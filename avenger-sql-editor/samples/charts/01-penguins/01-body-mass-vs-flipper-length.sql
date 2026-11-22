-- Body mass against flipper length, by species.
FROM samples.penguins
|> AGGREGATE mark_symbol(
     x => x(flipper_length_mm, scale => scale(zero => FALSE)),
     y => y(body_mass_g, scale => scale(zero => FALSE)),
     color => species,
   )
