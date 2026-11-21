-- A histogram of the body mass of the penguins of one species with the normal density of their mean
-- and standard deviation, scaled to the counts. The variable `chosen` names the species, and
-- `bin_width` gives both the width of the bins and the scale of the density, so the curve fits the
-- bars at any width. The density formula is the subtitle, and the label in the plot is Typst markup
-- that SQL builds from the fitted values.
DECLARE chosen STRING DEFAULT 'Gentoo';
DECLARE bin_width FLOAT64 DEFAULT 200;
FROM samples.penguins
|> WHERE species = chosen AND body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(body_mass_g, step => bin_width), title => 'Body mass (g)'),
     y => y(count(*), title => 'Penguins'),
     title => title(
       text => 'The body mass of the ' || chosen || ' penguins, with a normal fit',
       subtitle => '$display(f(x) = 1/(sigma sqrt(2 pi)) e^(-(x - mu)^2 / (2 sigma^2)))$',
     ),
   ) AS bars
|> CROSS JOIN (
     FROM samples.penguins
     |> WHERE species = chosen AND body_mass_g IS NOT NULL
     |> AGGREGATE avg(body_mass_g) AS mu, stddev(body_mass_g) AS sigma, count(*) AS n
     |> CROSS JOIN UNNEST(generate_array(0, 120)) AS i
     |> EXTEND 3800 + 20 * i AS x
     |> EXTEND n
          * bin_width
          * exp(-power(x - mu, 2) / (2 * power(sigma, 2)))
          / (sigma * sqrt(2 * 3.141592653589793)) AS density
     |> AGGREGATE mark_line(
          x => x(x, title => 'Body mass (g)'),
          y => y(density, title => 'Penguins'),
          color => value('#d62728'),
          stroke_width => value(2.5),
        ) AS curve
   )
|> CROSS JOIN (
     FROM samples.penguins
     |> WHERE species = chosen AND body_mass_g IS NOT NULL
     |> AGGREGATE avg(body_mass_g) AS mu, stddev(body_mass_g) AS sigma
     |> AGGREGATE mark_text(
          x => value(0.99),
          y => value(0.97),
          align => 'right',
          baseline => 'top',
          syntax => 'typst',
          size => value(14),
          text => '$mu = '
            || CAST(CAST(round(any_value(mu)) AS INT64) AS STRING)
            || ', quad sigma = '
            || CAST(CAST(round(any_value(sigma)) AS INT64) AS STRING)
            || '$',
        ) AS label
   )
|> SELECT layer(bars, curve, label) AS chart
