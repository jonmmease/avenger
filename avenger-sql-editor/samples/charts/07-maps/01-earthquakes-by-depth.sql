-- The earthquakes on an Equal Earth map, sized by magnitude and colored by depth. A sideways drag
-- turns the map around the poles, and the wheel zooms.
FROM samples.earthquakes
|> AGGREGATE mark_geo_symbol(
     longitude => longitude,
     latitude => latitude,
     size => size(magnitude, scale => scale(domain => [6, 9.5], range => [2, 120])),
     color => depth_km,
     coord => geo(projection => 'equal_earth', sphere => TRUE, graticule => TRUE),
     tools => [navigate()],
   )
