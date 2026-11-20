-- The earthquakes of magnitude 7 and larger on a globe of OpenStreetMap's tiles, with a tooltip of
-- the earthquake under the pointer. A drag turns the globe, the wheel zooms in to the streets, and
-- the tiles of each view load as it comes into view.
FROM samples.earthquakes
|> WHERE magnitude >= 7
|> AGGREGATE mark_geo_symbol(
     longitude => longitude,
     latitude => latitude,
     size => size(magnitude, scale => scale(domain => [7, 9.5], range => [20, 300])),
     color => color(
       depth_km,
       scale => scale(range => ['#f46d43', '#d53e4f', '#9e0142', '#5e3c99', '#2d004b']),
       title => 'Depth (km)',
     ),
     stroke => 'white',
     stroke_width => 0.5,
     coord => geo(
       projection => projection('orthographic', rotate => [170, -10], fit => 'whole_sphere'),
       sphere => TRUE,
       tiles => tiles('https://tile.openstreetmap.org/{z}/{x}/{y}.png'),
     ),
     attribution => attribution(credits => ['© OpenStreetMap contributors']),
     tools => [navigate(), tooltip()],
   )
