-- The mean bill length and depth of each species, as labels.
FROM samples.penguins
|> AGGREGATE mark_text(
     text => species,
     x => x(
       avg(bill_length_mm),
       scale => scale(domain => [35, 52]),
       title => 'Mean bill length (mm)',
     ),
     y => y(
       avg(bill_depth_mm),
       scale => scale(domain => [14, 20]),
       title => 'Mean bill depth (mm)',
     ),
     size => 16,
   )
