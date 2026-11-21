-- The countries, colored by continent.
FROM samples.countries
|> AGGREGATE mark_geo_shape(
     geometry => geometry,
     color => continent,
     coord => geo(projection => 'equal_earth', sphere => TRUE),
     tools => [navigate()],
   )
