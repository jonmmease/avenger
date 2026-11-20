-- The revenue of each quarter, stacked by the products with the most revenue, as many as the
-- variable `top`, and the rest as Other.
DECLARE top INT64 DEFAULT 2;
FROM samples.sales
|> AGGREGATE mark_bar(
     x => timeunit(month, unit => 'quarter'),
     y => stack(sum(revenue)),
     color => lump(product, n => top, weight => sum(revenue)),
   )
