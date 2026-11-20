-- `hconcat` puts its charts side by side and `vconcat` puts them one above another, in the order of
-- the arguments.
--
-- The bills and the sizes of the penguins, side by side.
FROM samples.penguins
|> AGGREGATE hconcat(
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species),
     mark_symbol(x => flipper_length_mm, y => body_mass_g, color => species),
   )
