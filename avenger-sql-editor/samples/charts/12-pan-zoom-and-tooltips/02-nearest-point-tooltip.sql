-- The same scatter plot, with a tooltip of the car nearest the pointer, within as many pixels as
-- the variable `radius`.
DECLARE radius FLOAT64 DEFAULT 20;
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => weight,
     y => mpg,
     color => origin,
     tools => [navigate(), tooltip(nearest => TRUE, max_radius => radius)],
   )
