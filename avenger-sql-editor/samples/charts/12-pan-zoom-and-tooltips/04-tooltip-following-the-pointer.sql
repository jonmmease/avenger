-- A tooltip that follows the pointer, without the swatch of the mark's color.
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => weight,
     y => mpg,
     color => origin,
     tools => [tooltip(anchor => 'pointer', hide_swatch => TRUE)],
   )
