FROM sales |> WHERE NOT (region = 'west' AND t >= TIMESTAMP '2024-03-01' AND t < TIMESTAMP '2024-05-01') |> EXTEND timeunit(t, unit => 'month') OVER () AS t_unit
|> AGGREGATE sum(units) AS sum_units
   GROUP BY t_unit, region
|> CALL impute(
     t_unit,
     [region],
     sum_units,
     value => 0,
     fill_gaps => true,
     flag => imputed)
|> EXTEND
     stack(sum_units) OVER (PARTITION BY t_unit ORDER BY region DESC NULLS FIRST, sum_units ASC NULLS LAST, imputed ASC NULLS LAST) AS sum_units_stack
|> AGGREGATE
     area(
       x => scaled(t_unit, title => 't'),
       y => scaled(sum_units_stack, title => 'sum(units)'),
       color => region,
       orient => 'vertical') AS area
