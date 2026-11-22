-- The standard normal density, shaded up to 1.96, with the integral of the cumulative distribution.
-- A sum over the generated rows gives the shaded area, which the label beside the bound reads.
FROM UNNEST(generate_array(0, 400)) AS i
|> EXTEND -4 + i / 50.0 AS z
|> EXTEND exp(-z * z / 2) / sqrt(2 * 3.141592653589793) AS phi
|> AGGREGATE mark_line(
     x => x(z, title => 'z'),
     y => y(phi, title => ''),
     color => value('#2b6ca3'),
     stroke_width => value(2.5),
   ) AS curve
|> CROSS JOIN (
     FROM UNNEST(generate_array(0, 298)) AS i
     |> EXTEND -4 + i / 50.0 AS z
     |> EXTEND exp(-z * z / 2) / sqrt(2 * 3.141592653589793) AS phi
     |> AGGREGATE mark_area(
          x => x(z, title => 'z'),
          y => y(phi, title => ''),
          color => value('#a6c8e4'),
        ) AS shaded
   )
|> CROSS JOIN (
     FROM UNNEST(generate_array(0, 298)) AS i
     |> EXTEND -4 + i / 50.0 AS z
     |> EXTEND exp(-z * z / 2) / sqrt(2 * 3.141592653589793) AS phi
     |> AGGREGATE (sum(phi) - max(CASE WHEN i = 298 THEN phi END) / 2) / 50 AS p
     |> EXTEND 1.96 AS z
     |> AGGREGATE
          mark_rule(
            x => x(z, title => 'z'),
            color => value('#555555'),
            stroke_dasharray => value('4 3'),
          ) AS bound,
          mark_text(
            x => value(0.76),
            y => value(0.45),
            align => 'left',
            syntax => 'typst',
            size => value(15),
            text => '$Phi(1.96) approx ' || CAST(round(any_value(p), 3) AS STRING) || '$',
          ) AS value,
   )
|> SELECT layer(
     shaded,
     curve,
     bound,
     value,
     title => title(
       text => 'The standard normal distribution',
       subtitle => '$display(Phi(z) = 1/sqrt(2 pi) integral_(-infinity)^z e^(-t^2 / 2) dif t)$',
     ),
   ) AS chart
