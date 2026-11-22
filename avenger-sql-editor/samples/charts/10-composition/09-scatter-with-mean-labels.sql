-- The penguins, with a label at the mean bill of each species.
FROM samples.penguins
|> AGGREGATE layer(
     mark_symbol(
       x => x(
         bill_length_mm,
         scale => scale(zero => FALSE),
         title => 'Bill length (mm)',
       ),
       y => y(bill_depth_mm, scale => scale(zero => FALSE), title => 'Bill depth (mm)'),
       color => species,
       opacity => 0.4,
     ),
     mark_text(
       x => x(avg(bill_length_mm), title => 'Bill length (mm)'),
       y => y(avg(bill_depth_mm), title => 'Bill depth (mm)'),
       text => species,
       size => 14,
     ),
   )
