-- Bill depth against bill length, with a shape for each island and axis titles.
FROM samples.penguins
|> WHERE bill_length_mm IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE), title => 'Bill length (mm)'),
     y => y(bill_depth_mm, scale => scale(zero => FALSE), title => 'Bill depth (mm)'),
     color => species,
     shape => island,
     size => 60,
   )
