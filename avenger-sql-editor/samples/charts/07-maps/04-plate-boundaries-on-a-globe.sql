-- The boundaries of the tectonic plates on an orthographic globe over the Pacific, with the
-- subduction zones apart. A drag turns the globe, and the wheel zooms.
FROM (
  FROM samples.countries
  |> AGGREGATE mark_geo_shape(
       geometry => geometry,
       fill => 'gray',
       fill_opacity => 0.4,
       coord => geo(
         projection => projection('orthographic', rotate => [170, -10]),
         sphere => TRUE,
         graticule => TRUE,
       ),
     ) AS land
)
|> CROSS JOIN (
     FROM samples.plate_boundaries
     |> AGGREGATE mark_geo_shape(
          geometry => geometry,
          stroke => stroke(if(subduction, 'Subduction zone', 'Other'), title => 'Boundary'),
          stroke_width => 2,
          coord => geo(
            projection => projection('orthographic', rotate => [170, -10]),
            sphere => TRUE,
            graticule => TRUE,
          ),
        ) AS plates
   )
|> SELECT layer(land, plates, tools => [navigate()]) AS chart
