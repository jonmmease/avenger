FROM penguins |> WHERE body_mass_g IS NOT NULL |> AGGREGATE count(*) AS penguins, avg(body_mass_g) AS mean_mass
   GROUP BY island
|> AS aggregates
|> LEFT OUTER JOIN (
     FROM penguins
     |> WHERE body_mass_g IS NOT NULL
     |> EXTEND bin(body_mass_g) OVER () AS body_mass_g_bin
     |> AGGREGATE count(*) AS n
        GROUP BY island, body_mass_g_bin
     |> AGGREGATE
          bar(
            x => scaled(body_mass_g_bin, title => 'body_mass_g'),
            y => scaled(n, title => 'count(*)')) AS chart
        GROUP BY island
   ) AS charts ON aggregates.island IS NOT DISTINCT FROM charts.island
|> SELECT
     aggregates.island,
     charts.chart,
     aggregates.penguins,
     aggregates.mean_mass |> ORDER BY island
