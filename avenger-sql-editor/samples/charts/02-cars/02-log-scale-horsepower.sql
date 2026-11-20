-- Horsepower on a log scale against displacement.
FROM samples.cars
|> WHERE horsepower IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(displacement, scale => scale(type => 'log')),
     y => y(horsepower, scale => scale(type => 'log')),
     color => origin,
   )
