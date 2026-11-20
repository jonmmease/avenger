-- Miles per gallon against weight, by origin, sized by the number of cylinders.
FROM samples.cars
|> AGGREGATE mark_symbol(x => weight, y => mpg, color => origin, size => cylinders, opacity => 0.7)
