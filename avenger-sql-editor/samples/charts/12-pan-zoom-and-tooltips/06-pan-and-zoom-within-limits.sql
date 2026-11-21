-- Pan and zoom that stay inside the data, with a smallest span of 200 lb and 2 miles per gallon.
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => x(
       weight,
       scale => scale(navigation => navigation('data', min_span => 200)),
     ),
     y => y(
       mpg,
       scale => scale(navigation => navigation('data', min_span => 2)),
     ),
     color => origin,
     tools => [navigate()],
   )
