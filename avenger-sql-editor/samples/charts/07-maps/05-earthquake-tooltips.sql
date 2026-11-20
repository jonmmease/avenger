-- The largest earthquakes, from magnitude 7.5, with a tooltip of the earthquake under the pointer,
-- and pan and zoom.
FROM samples.earthquakes
|> WHERE magnitude >= 7.5
|> AGGREGATE mark_geo_symbol(
     longitude => longitude,
     latitude => latitude,
     size => size(magnitude, scale => scale(domain => [7.5, 9.5], range => [20, 300])),
     color => depth_km,
     coord => geo(projection => 'natural_earth1', sphere => TRUE, graticule => TRUE),
     tools => [navigate(), tooltip()],
   )
