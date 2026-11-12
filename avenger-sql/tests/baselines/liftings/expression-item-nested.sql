FROM penguins |> WHERE body_mass_g IS NOT NULL |> AGGREGATE
     symbol(x => bill_length_mm, y => bill_depth_mm) AS chart_symbol
   GROUP BY island
|> AS aggregates
|> LEFT OUTER JOIN (
     FROM penguins
     |> WHERE body_mass_g IS NOT NULL
     |> AGGREGATE avg(body_mass_g) AS avg_body_mass_g
        GROUP BY island, species
     |> AGGREGATE
          bar(
            x => species,
            y => scaled(avg_body_mass_g, title => 'avg(body_mass_g)')) AS chart_bar
        GROUP BY island
   ) AS charts ON aggregates.island IS NOT DISTINCT FROM charts.island
|> LEFT OUTER JOIN (
     FROM penguins
     |> WHERE body_mass_g IS NOT NULL
     |> AGGREGATE avg(body_mass_g) AS avg_body_mass_g
        GROUP BY island
     |> AGGREGATE
          rule(y => scaled(avg_body_mass_g, title => 'avg(body_mass_g)')) AS chart_rule
        GROUP BY island
   ) AS charts_2 ON aggregates.island IS NOT DISTINCT FROM charts_2.island
|> SELECT
     aggregates.island,
     hconcat(
       layer(charts.chart_bar, charts_2.chart_rule),
       aggregates.chart_symbol,
       layout => concat_layout(gap => 24)) AS chart |> ORDER BY island
