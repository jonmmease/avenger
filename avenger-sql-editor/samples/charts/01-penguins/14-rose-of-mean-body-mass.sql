-- The mean body mass of each species as a rose.
FROM samples.penguins
|> AGGREGATE mark_polar_bar(
     theta => species,
     radius => radius(avg(body_mass_g), scale => scale(zero => TRUE)),
     color => species,
   )
