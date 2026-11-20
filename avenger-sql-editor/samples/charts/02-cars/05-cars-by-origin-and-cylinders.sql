-- The cars of each origin and number of cylinders.
FROM samples.cars
|> AGGREGATE mark_rect(
     x => origin,
     y => CAST(cylinders AS STRING),
     color => color(count(*), scale => scale(range => 'viridis')),
   )
