FROM penguins |> WHERE body_mass_g IS NOT NULL |> AGGREGATE count(*) AS n
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
            y => scaled(avg_body_mass_g, title => 'avg(body_mass_g)')) AS layer_bar
        GROUP BY island
   ) AS charts ON aggregates.island IS NOT DISTINCT FROM charts.island
|> LEFT OUTER JOIN (
     FROM penguins
     |> WHERE body_mass_g IS NOT NULL
     |> AGGREGATE avg(body_mass_g) AS avg_body_mass_g
        GROUP BY island
     |> AGGREGATE
          rule(y => scaled(avg_body_mass_g, title => 'avg(body_mass_g)')) AS layer_rule
        GROUP BY island
   ) AS charts_2 ON aggregates.island IS NOT DISTINCT FROM charts_2.island
|> SELECT
     aggregates.island,
     aggregates.n,
     layer(charts.layer_bar, charts_2.layer_rule) AS layer |> ORDER BY island
