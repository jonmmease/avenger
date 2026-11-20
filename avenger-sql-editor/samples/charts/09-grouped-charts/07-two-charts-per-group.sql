-- Two charts for each species and island, the bills and the sizes, from an `AGGREGATE` with two
-- mark functions.
FROM samples.penguins
|> AGGREGATE
     count(*) AS n,
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => sex) AS bills,
     mark_symbol(x => flipper_length_mm, y => body_mass_g, color => sex) AS sizes,
   GROUP BY species, island
|> ORDER BY species, island
