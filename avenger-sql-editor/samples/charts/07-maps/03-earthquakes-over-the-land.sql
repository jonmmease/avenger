-- The earthquakes over the countries. Each table's chart is a column of one row, the `CROSS JOIN`
-- puts the two in one row, and `layer` draws them on one map, as their projections are equal.
FROM (
  FROM samples.countries
  |> AGGREGATE mark_geo_shape(
       geometry => geometry,
       fill => 'gray',
       fill_opacity => 0.4,
       coord => geo(projection => 'equal_earth', sphere => TRUE),
     ) AS land
)
|> CROSS JOIN (
     FROM samples.earthquakes
     |> AGGREGATE mark_geo_symbol(
          longitude => longitude,
          latitude => latitude,
          size => size(magnitude, scale => scale(domain => [6, 9.5], range => [2, 120])),
          color => depth_km,
          coord => geo(projection => 'equal_earth', sphere => TRUE),
        ) AS quakes
   )
|> SELECT layer(land, quakes, tools => [navigate()]) AS chart
