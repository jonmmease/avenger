-- Miles per gallon against weight for each number of cylinders, with a facet title.
FROM samples.cars
|> AGGREGATE mark_symbol(x => weight, y => mpg, color => origin) AS chart GROUP BY cylinders
|> AGGREGATE facet_wrap(chart, cylinders, facet => facet(title => 'Cylinders')) AS chart
