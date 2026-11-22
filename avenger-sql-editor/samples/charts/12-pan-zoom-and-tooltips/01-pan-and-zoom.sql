-- Miles per gallon against weight, with pan and zoom.
FROM samples.cars
|> AGGREGATE mark_symbol(x => weight, y => mpg, color => origin, tools => [navigate()])
