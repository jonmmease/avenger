FROM penguins
|> AGGREGATE AVG(body_mass_g) AS `AVG(body_mass_g)` GROUP BY island, species
|> SELECT `island`, `species`, `AVG(body_mass_g)`
|> AGGREGATE symbol(
     x => x(`species`, title => 'Species', axis => axis(grid => FALSE)),
     y => y(
       `AVG(body_mass_g)`,
       scale => scale(zero => FALSE),
       axis => axis(label_rotation => -45, title => title('Mean body mass (g)', font_size => 14)),
     ),
     color => `species`,
     tools => [navigate(), tooltip(nearest => TRUE, max_radius => 20)],
     ORDER BY `species`
   ) AS `chart`
   GROUP BY `island`
