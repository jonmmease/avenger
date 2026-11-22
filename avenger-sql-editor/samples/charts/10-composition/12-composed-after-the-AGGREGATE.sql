-- The same charts as named columns, composed after the `AGGREGATE`.
FROM samples.penguins
|> AGGREGATE
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species) AS bills,
     mark_symbol(x => flipper_length_mm, y => body_mass_g, color => species) AS sizes,
|> SELECT hconcat(bills, sizes) AS chart
