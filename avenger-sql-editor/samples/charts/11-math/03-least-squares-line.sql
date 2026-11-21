-- Miles per gallon against weight with the least-squares line of `regr_intercept` and `regr_slope`,
-- the estimator of the slope as the subtitle, and the fitted line and its R² as markup that SQL
-- builds.
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => x(weight, title => 'Weight (lb)', scale => scale(zero => FALSE)),
     y => y(mpg, title => 'Miles per gallon'),
     color => value('#9aa7b8'),
   ) AS points
|> CROSS JOIN (
     FROM samples.cars
     |> WHERE mpg IS NOT NULL AND weight IS NOT NULL
     |> AGGREGATE
          avg(weight) AS mx,
          avg(mpg) AS my,
          avg(weight * mpg) AS mxy,
          avg(weight * weight) AS mxx,
          min(weight) AS low,
          max(weight) AS high,
     |> EXTEND (mxy - mx * my) / (mxx - mx * mx) AS b
     |> EXTEND my - b * mx AS a
     |> CROSS JOIN UNNEST([0, 1]) AS e
     |> EXTEND CASE WHEN e = 0 THEN low ELSE high END AS x
     |> AGGREGATE mark_line(
          x => x(x, title => 'Weight (lb)'),
          y => y(a + b * x, title => 'Miles per gallon'),
          color => value('#d62728'),
          stroke_width => value(2.5),
        ) AS fitted
   )
|> CROSS JOIN (
     FROM samples.cars
     |> WHERE mpg IS NOT NULL AND weight IS NOT NULL
     |> AGGREGATE
          avg(weight) AS mx,
          avg(mpg) AS my,
          avg(weight * mpg) AS mxy,
          avg(weight * weight) AS mxx,
          avg(mpg * mpg) AS myy,
     |> EXTEND (mxy - mx * my) / (mxx - mx * mx) AS b
     |> EXTEND my - b * mx AS a, pow(mxy - mx * my, 2) / ((mxx - mx * mx) * (myy - my * my)) AS r2
     |> AGGREGATE mark_text(
          x => value(0.98),
          y => value(0.97),
          align => 'right',
          baseline => 'top',
          syntax => 'typst',
          size => value(15),
          text => '$hat(y) = '
            || CAST(round(any_value(a), 2) AS STRING)
            || ' - '
            || CAST(round(-any_value(b), 5) AS STRING)
            || ' x, quad R^2 = '
            || CAST(round(any_value(r2), 2) AS STRING)
            || '$',
        ) AS label
   )
|> SELECT layer(
     points,
     fitted,
     label,
     title => title(
       text => 'Miles per gallon against weight, with a least-squares line',
       subtitle => '$display(hat(beta)_1 = (sum_(i=1)^n (x_i - macron(x))(y_i - macron(y))) / (sum_(i=1)^n (x_i - macron(x))^2))$',
     ),
   ) AS chart
