-- A streamgraph of the units of each product, from the offset `center` of the variable `offset`,
-- which `'zero'` or `'normalize'` changes to stacked or normalized areas.
DECLARE offset STRING DEFAULT 'center';
FROM samples.sales
|> AGGREGATE mark_area(x => month, y => stack(sum(units), offset => offset), color => product)
