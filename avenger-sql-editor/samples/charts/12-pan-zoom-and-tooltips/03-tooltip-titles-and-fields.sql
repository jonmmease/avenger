-- A tooltip with the origin as its title and fields with titles of their own.
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => weight,
     y => mpg,
     color => origin,
     size => cylinders,
     tools => [
       tooltip(
         nearest => TRUE,
         max_radius => 20,
         title => 'origin',
         fields => [
           field(column => 'weight', title => 'Weight (lb)'),
           field(column => 'mpg', title => 'Miles per gallon'),
           field(column => 'cylinders', title => 'Cylinders'),
         ],
       ),
     ],
   )
