-- The mean horsepower and miles per gallon of each origin, connected from year to year by the
-- `order` channel.
FROM samples.cars
|> AGGREGATE mark_line(
     x => x(avg(horsepower), scale => scale(zero => FALSE), title => 'Horsepower'),
     y => y(avg(mpg), scale => scale(zero => FALSE), title => 'Miles per gallon'),
     color => origin,
     order => year,
   )
