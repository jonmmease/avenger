-- The revenue of each month, stacked by region.
FROM samples.sales
|> AGGREGATE mark_area(x => month, y => y(stack(sum(revenue)), title => 'Revenue'), color => region)
