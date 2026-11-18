# Example queries

The editor's Examples tab shows a card for each example of this file, under its `###` title, and a click puts its query in the editor and runs it. A query copied into the editor runs with ⌘↵. Each query in the first eight sections returns one row with one chart column, so the editor shows it as an interactive chart that fills the result pane. The queries in "Grouped charts" return a chart for each group, in a table. The queries in "Composition" lay out several charts as one chart: facets, layers, and concats. The queries in "Pan, zoom, and tooltips" declare tools.

Most queries end in a pipe `AGGREGATE` whose item is a mark call, which builds a chart of the rows before it:

- A channel that holds an aggregate, such as `y => count(*)`, is a measure. The other channels are keys, and the chart has one mark for each combination of the keys. A call without aggregates draws one mark for each row.
- The transforms are written inline. A window that reads only rows, such as `bin`, `timeunit`, `lump`, or `NTILE(4) OVER (ORDER BY x)`, computes before the aggregates, and a window that reads an aggregate, such as `stack(count(*))` or `sum(count(*)) OVER panel`, reads the aggregated rows. `panel`, `position`, and `series` name the windows of the call's groups, positions, and series.
- A `mark_line` or a `mark_area` connects its points along its key, `x` or `y`, or in the order of its `order` channel. A stacked `mark_area` or `mark_line` fills the keys that a series lacks with zero. A `|> CALL impute(...)` before the `AGGREGATE` adds the rows that the groups of a staged query lack.
- An axis title is the text of its expression, such as `count(*)`. A channel function's `title`, as `y(..., title => '...')`, sets another title.
- `GROUP BY` draws one chart for each group. The result has the group columns and a column for each chart, named by its alias, or without one by its mark function, such as `mark_bar`. Another item beside the mark call, such as `count(*) AS penguins`, is an aggregate of each group, and several mark calls draw a chart of each. An item can compose its mark calls, as in `layer(mark_bar(...), mark_rule(...))`.
- The Lifted SQL tab shows the query with each mark call that computes its statistics written as the stages that compute them, as a query that you could type, with the text that it adds highlighted.

The scalar compositions `layer`, `hconcat`, and `vconcat` compose the charts of a row, such as the mark calls of an item. The aggregates `layer_agg`, `hconcat_agg`, `vconcat_agg`, `facet_row`, `facet_col`, and `facet_wrap` compose the charts of a group, in a pipe `AGGREGATE`.

The editor holds the nine datasets as the tables of the schema `samples`: `samples.penguins`, `samples.cars`, `samples.movies`, `samples.iris`, `samples.seattle_weather`, `samples.sales`, `samples.earthquakes`, `samples.countries`, and `samples.plate_boundaries`. [`samples/README.md`](samples/README.md) gives the source and the terms of each.

## Penguins

### Body mass vs. flipper length

Body mass against flipper length, by species.

```sql
FROM samples.penguins
|> AGGREGATE mark_symbol(
     x => x(flipper_length_mm, scale => scale(zero => FALSE)),
     y => y(body_mass_g, scale => scale(zero => FALSE)),
     color => species,
   )
```

### Bill shapes by island

Bill depth against bill length, with a shape for each island and axis titles.

```sql
FROM samples.penguins
|> WHERE bill_length_mm IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE), title => 'Bill length (mm)'),
     y => y(bill_depth_mm, scale => scale(zero => FALSE), title => 'Bill depth (mm)'),
     color => species,
     shape => island,
     size => 60,
   )
```

### Penguins per species

The number of penguins of each species.

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(x => species, y => count(*))
```

### Mean mass per island

The mean body mass on each island, as horizontal bars from the heaviest.

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => x(avg(body_mass_g), title => 'Mean body mass (g)'),
     y => island,
     ORDER BY avg(body_mass_g) DESC
   )
```

### Share of each island

The share of the penguins on each island, from a window of the counts.

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => island,
     y => y(CAST(count(*) AS FLOAT64) / sum(count(*)) OVER (), title => 'Share of penguins'),
   )
```

### Species stacked by sex

The penguins of each species, stacked by sex.

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(x => species, y => stack(count(*)), color => sex)
```

### Stacked, largest at bottom

The penguins of each species, stacked by island with the largest group at the bottom.

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => species,
     y => stack(count(*)) OVER (position ORDER BY count(*) DESC),
     color => island,
   )
```

### Normalized stacked bars

The share of each island in the penguins of each species, from the offset `normalize`.

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => species,
     y => y(stack(count(*), offset => 'normalize'), title => 'Share'),
     color => island,
   )
```

### Body mass histogram

A histogram of body mass.

```sql
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(body_mass_g), title => 'Body mass (g)'),
     y => y(count(*), title => 'Penguins'),
   )
```

### Stacked flipper histogram

A histogram of flipper length, stacked by species, in at most as many bins as the variable `bins`.

```sql
DECLARE bins FLOAT64 DEFAULT 20;
FROM samples.penguins
|> WHERE flipper_length_mm IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(flipper_length_mm, maxbins => bins), title => 'Flipper length (mm)'),
     y => y(stack(count(*)), title => 'Penguins'),
     color => species,
   )
```

### Heat map with imputed zeros

The penguins of each species and island as a heat map, with 0 for the pairs that have none, from a pipe `CALL` of the table function `impute`.

```sql
FROM samples.penguins
|> AGGREGATE count(*) AS n GROUP BY species, island
|> CALL impute(island, [species], n, value => 0)
|> AGGREGATE mark_rect(x => island, y => species, color => n)
```

### Mean bills as labels

The mean bill length and depth of each species, as labels.

```sql
FROM samples.penguins
|> AGGREGATE mark_text(
     text => species,
     x => x(
       avg(bill_length_mm),
       scale => scale(domain => [35, 52]),
       title => 'Mean bill length (mm)',
     ),
     y => y(
       avg(bill_depth_mm),
       scale => scale(domain => [14, 20]),
       title => 'Mean bill depth (mm)',
     ),
     size => 16,
   )
```

### Donut of species

The penguins of each species as a donut.

```sql
FROM samples.penguins
|> AGGREGATE mark_polar_bar(
     theta => stack(count(*)),
     color => species,
     coord => polar(inner_radius => 0.5),
   )
```

### Rose of mean body mass

The mean body mass of each species as a rose.

```sql
FROM samples.penguins
|> AGGREGATE mark_polar_bar(
     theta => species,
     radius => radius(avg(body_mass_g), scale => scale(zero => TRUE)),
     color => species,
   )
```

### Histogram with tools

A histogram with a title, a navigation, and tooltips.

```sql
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => bin(body_mass_g, maxbins => 20),
     y => stack(count(*)),
     color => species,
     title => 'Body mass of the penguins',
     tools => [navigate(), tooltip()],
   )
```

## Cars

### MPG vs. weight by origin

Miles per gallon against weight, by origin, sized by the number of cylinders.

```sql
FROM samples.cars
|> AGGREGATE mark_symbol(x => weight, y => mpg, color => origin, size => cylinders, opacity => 0.7)
```

### Log-scale horsepower

Horsepower on a log scale against displacement.

```sql
FROM samples.cars
|> WHERE horsepower IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(displacement, scale => scale(type => 'log')),
     y => y(horsepower, scale => scale(type => 'log')),
     color => origin,
   )
```

### Mean MPG per year

The mean miles per gallon of each model year, by origin.

```sql
FROM samples.cars
|> AGGREGATE mark_line(x => year, y => avg(mpg), color => origin)
```

### Connected scatter by year

The mean horsepower and miles per gallon of each origin, connected from year to year by the `order` channel.

```sql
FROM samples.cars
|> AGGREGATE mark_line(
     x => x(avg(horsepower), scale => scale(zero => FALSE), title => 'Horsepower'),
     y => y(avg(mpg), scale => scale(zero => FALSE), title => 'Miles per gallon'),
     color => origin,
     order => year,
   )
```

### Cars by origin and cylinders

The cars of each origin and number of cylinders.

```sql
FROM samples.cars
|> AGGREGATE mark_rect(
     x => origin,
     y => CAST(cylinders AS STRING),
     color => color(count(*), scale => scale(range => 'viridis')),
   )
```

### Stacked MPG histogram

A histogram of miles per gallon, stacked by origin.

```sql
FROM samples.cars
|> WHERE mpg IS NOT NULL
|> AGGREGATE mark_bar(x => bin(mpg), y => stack(count(*)), color => origin)
```

## Movies

### IMDB vs. Rotten Tomatoes

The IMDB rating against the Rotten Tomatoes rating.

```sql
FROM samples.movies
|> AGGREGATE mark_symbol(x => rotten_tomatoes_rating, y => imdb_rating, opacity => 0.3, size => 20)
```

### Top genres with lump

The movies of each genre, with the largest genres, as many as the variable `top`, and the rest as Other, from `lump`.

```sql
DECLARE top INT64 DEFAULT 5;
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_bar(x => count(*), y => lump(genre, n => top) ORDER BY count(*) DESC)
```

### Movies per five years

The movies of each five years, stacked by the four largest genres.

```sql
FROM samples.movies
|> WHERE genre IS NOT NULL AND year IS NOT NULL
|> AGGREGATE mark_bar(x => bin(year, step => 5), y => stack(count(*)), color => lump(genre, n => 4))
```

### Movies per year, time axis

The movies of each year, on a time axis.

```sql
FROM samples.movies
|> WHERE year IS NOT NULL
|> AGGREGATE mark_area(x => x(date(year, 1, 1), title => 'Year'), y => count(*))
```

### Stacked area of genres

The movies of each year, stacked by the four genres with the most movies and the rest as Other. A stacked area imputes 0 for the years without movies of a genre.

```sql
FROM samples.movies
|> WHERE year IS NOT NULL AND genre IS NOT NULL
|> AGGREGATE mark_area(
     x => x(date(year, 1, 1), title => 'Year'),
     y => stack(count(*)),
     color => lump(genre, n => 4),
   )
```

## Iris

### Petal width vs. length

Petal width against petal length, by species.

```sql
FROM samples.iris
|> AGGREGATE mark_symbol(x => `PetalLengthCm`, y => `PetalWidthCm`, color => `Species`)
```

## Seattle weather

### Daily high temperature

The highest temperature of each day.

```sql
FROM samples.seattle_weather
|> AGGREGATE mark_line(x => date, y => temp_max)
```

### Monthly temperature range

The range of temperatures of each month, as rules from the lowest to the highest.

```sql
FROM samples.seattle_weather
|> AGGREGATE mark_rule(
     x => x(timeunit(date, unit => 'month').start, title => 'Month'),
     y => y(min(temp_min), title => 'Temperature (°C)'),
     y2 => max(temp_max),
     stroke_width => 4,
   )
```

### Monthly precipitation

The precipitation of each unit of time, such as each month, as bars that span their units, from `timeunit` with the unit of the variable `unit`.

```sql
DECLARE unit STRING DEFAULT 'month';
FROM samples.seattle_weather
|> AGGREGATE mark_bar(x => timeunit(date, unit => unit), y => sum(precipitation))
```

### Weather days per month

The days of each kind of weather in each month.

```sql
FROM samples.seattle_weather
|> AGGREGATE mark_bar(x => timeunit(date, unit => 'month'), y => stack(count(*)), color => weather)
```

### Weather as a stacked area

The same days as a stacked area. Snow falls in 7 of the 48 months, and the stacked area imputes 0 days of snow in the other months.

```sql
FROM samples.seattle_weather
|> AGGREGATE mark_area(x => timeunit(date, unit => 'month'), y => stack(count(*)), color => weather)
```

### Weather lines, imputed zeros

The days of each kind of weather in each month, as lines with 0 for the months without that weather, from `impute => 0`.

```sql
FROM samples.seattle_weather
|> AGGREGATE count(*) AS n GROUP BY date_trunc(date, MONTH) AS month, weather
|> CALL impute(month, [weather], n, value => 0)
|> AGGREGATE mark_line(
     x => x(month, title => 'Month'),
     y => y(n, title => 'count(*)'),
     color => weather,
   )
```

### Weather lines with gaps

The same lines with a break at each month without that weather, from `impute => impute(method => 'gap')`.

```sql
FROM samples.seattle_weather
|> AGGREGATE count(*) AS n GROUP BY date_trunc(date, MONTH) AS month, weather
|> CALL impute(month, [weather], n, value => NULL)
|> AGGREGATE mark_line(
     x => x(month, title => 'Month'),
     y => y(n, title => 'count(*)'),
     color => weather,
   )
```

### Weather kinds, most first

The days of each kind of weather, from the most.

```sql
FROM samples.seattle_weather
|> AGGREGATE mark_bar(x => weather, y => count(*), color => weather ORDER BY count(*) DESC)
```

### Wind vs. high temperature

The wind against the highest temperature, by weather.

```sql
FROM samples.seattle_weather
|> AGGREGATE mark_symbol(x => temp_max, y => wind, color => weather, opacity => 0.5)
```

## Sales

### Revenue stacked by region

The revenue of each month, stacked by region.

```sql
FROM samples.sales
|> AGGREGATE mark_area(x => month, y => y(stack(sum(revenue)), title => 'Revenue'), color => region)
```

### Normalized regional revenue

The share of each region in the revenue of each month, from the offset `normalize`.

```sql
FROM samples.sales
|> AGGREGATE mark_bar(
     x => timeunit(month, unit => 'month'),
     y => y(stack(sum(revenue), offset => 'normalize'), title => 'Share of revenue'),
     color => region,
   )
```

### Streamgraph of units

A streamgraph of the units of each product, from the offset `center` of the variable `offset`, which `'zero'` or `'normalize'` changes to stacked or normalized areas.

```sql
DECLARE offset STRING DEFAULT 'center';
FROM samples.sales
|> AGGREGATE mark_area(x => month, y => stack(sum(units), offset => offset), color => product)
```

### Units stacked by channel

The units of each product, stacked by channel.

```sql
FROM samples.sales
|> AGGREGATE mark_bar(x => product, y => y(stack(sum(units)), title => 'Units'), color => channel)
```

### Quarterly revenue, top two

The revenue of each quarter, stacked by the products with the most revenue, as many as the variable `top`, and the rest as Other.

```sql
DECLARE top INT64 DEFAULT 2;
FROM samples.sales
|> AGGREGATE mark_bar(
     x => timeunit(month, unit => 'quarter'),
     y => stack(sum(revenue)),
     color => lump(product, n => top, weight => sum(revenue)),
   )
```

## Maps

A geo mark draws on a map: `mark_geo_symbol` at the `longitude` and `latitude` of each row, and `mark_geo_shape` of the GeoArrow `geometry` of each row, such as a country or a line. Its `coord => geo(...)` names its projection, `sphere` and `graticule` draw the globe behind the data and its lines of longitude and latitude, and `tiles` draws a street map under the data. With `navigate()` among its tools, a drag pans the map, or turns a globe, and the wheel zooms. `samples.earthquakes` holds the earthquakes of magnitude 6 and larger from 2000 through 2024, `samples.countries` the outlines of the countries, and `samples.plate_boundaries` the boundaries of the tectonic plates.

### Earthquakes by depth

The earthquakes on an Equal Earth map, sized by magnitude and colored by depth. A sideways drag turns the map around the poles, and the wheel zooms.

```sql
FROM samples.earthquakes
|> AGGREGATE mark_geo_symbol(
     longitude => longitude,
     latitude => latitude,
     size => size(magnitude, scale => scale(domain => [6, 9.5], range => [2, 120])),
     color => depth_km,
     coord => geo(projection => 'equal_earth', sphere => TRUE, graticule => TRUE),
     tools => [navigate()],
   )
```

### Countries by continent

The countries, colored by continent.

```sql
FROM samples.countries
|> AGGREGATE mark_geo_shape(
     geometry => geometry,
     color => continent,
     coord => geo(projection => 'equal_earth', sphere => TRUE),
     tools => [navigate()],
   )
```

### Earthquakes over the land

The earthquakes over the countries. Each table's chart is a column of one row, the `CROSS JOIN` puts the two in one row, and `layer` draws them on one map, as their projections are equal.

```sql
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
```

### Plate boundaries on a globe

The boundaries of the tectonic plates on an orthographic globe over the Pacific, with the subduction zones apart. A drag turns the globe, and the wheel zooms.

```sql
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
```

### Earthquake tooltips

The largest earthquakes, from magnitude 7.5, with a tooltip of the earthquake under the pointer, and pan and zoom.

```sql
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
```

### Earthquakes on a street map

The earthquakes of magnitude 7 and larger on a globe of OpenStreetMap's tiles, with a tooltip of the earthquake under the pointer. A drag turns the globe, the wheel zooms in to the streets, and the tiles of each view load as it comes into view.

```sql
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
```

### Earthquakes per year

The number of earthquakes in each year, stacked by depth, where shallow earthquakes are less than 70 km deep.

```sql
FROM samples.earthquakes
|> AGGREGATE mark_bar(
     x => x(timeunit(time, unit => 'year'), title => 'Year'),
     y => y(stack(count(*)), title => 'Earthquakes'),
     color => color(if(depth_km < 70, 'Shallow', 'Deep'), title => 'Depth'),
   )
```

## Variables

A `DECLARE` before the query declares a script variable, which the query reads by its name. The plan holds a placeholder of each variable, and the pipeline computes the values of the variables and binds them before the query runs, so a variable can hold the result of a query. A variable stands in the options of a mark, such as `title`, and of a transform, such as the `step` of `bin`.

### A threshold in a variable

The penguins of each species, stacked by whether they reach the body mass in the variable `heavy`, which the title states too.

```sql
DECLARE heavy INT64 DEFAULT 4500;
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> EXTEND if(body_mass_g >= heavy, 'heavy', 'light') AS class
|> AGGREGATE mark_bar(
     x => species,
     y => stack(count(*)),
     color => class,
     title => 'Penguins of at least ' || CAST(heavy AS STRING) || ' g',
   )
```

### A date range in variables

The highest temperature of each day in the six months from the start date, from two variables.

```sql
DECLARE start_date DATE DEFAULT DATE '2014-01-01';
DECLARE months INT64 DEFAULT 6;
FROM samples.seattle_weather
|> WHERE date >= start_date AND date < date_add(start_date, INTERVAL months MONTH)
|> AGGREGATE mark_line(x => date, y => y(temp_max, title => 'Highest temperature (°C)'))
```

### A bin width in a variable

A histogram of body mass whose bins are as wide as the variable `bin_width`, which the title states too.

```sql
DECLARE bin_width FLOAT64 DEFAULT 250;
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(body_mass_g, step => bin_width), title => 'Body mass (g)'),
     y => stack(count(*)),
     color => species,
     title => 'Body mass in bins of ' || CAST(bin_width AS STRING) || ' g',
   )
```

### Genres up to a count

The ratings of the movies, colored by as many of the largest genres as the variable `genres` names, and the rest as Other.

```sql
DECLARE genres INT64 DEFAULT 5;
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_symbol(
     x => rotten_tomatoes_rating,
     y => imdb_rating,
     color => lump(genre, n => genres),
     opacity => 0.4,
   )
```

### A variable from a query

The mean body mass of each species above the mean of every penguin, which a query computes into the variable `mean_mass`.

```sql
DECLARE mean_mass FLOAT64 DEFAULT (
  FROM samples.penguins
  |> AGGREGATE avg(body_mass_g)
);
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => species,
     y => y(avg(body_mass_g) - mean_mass, title => 'Mean body mass above the mean (g)'),
   )
```

## Grouped charts

A mark call with `GROUP BY` builds a chart for each group, so these queries return a table with a row for each group. Drag a column edge or a row edge to resize the charts, check "Interactive charts" to pan and zoom them in place, and double-click a chart to expand it.

### Scatter for each species

Body mass against flipper length for each species.

```sql
FROM samples.penguins
|> AGGREGATE mark_symbol(
     x => x(flipper_length_mm, scale => scale(zero => FALSE)),
     y => y(body_mass_g, scale => scale(zero => FALSE)),
     color => sex,
   )
   GROUP BY species
|> ORDER BY species
```

### Bills with a count item

Bill depth against bill length on each island, with the number of penguins of each island from an aggregate item.

```sql
FROM samples.penguins
|> AGGREGATE
     count(*) AS penguins,
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species) AS chart,
   GROUP BY island
|> ORDER BY penguins DESC
```

### Histograms with shared bins

A histogram of body mass on each island, in at most as many bins as the variable `bins`. The bins are those of all the penguins, so every histogram has the same bin edges.

```sql
DECLARE bins FLOAT64 DEFAULT 20;
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(x => bin(body_mass_g, maxbins => bins), y => count(*)) GROUP BY island
|> ORDER BY island
```

### Histograms with panel bins

The same histograms, with the bins of each island, from `OVER panel`.

```sql
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(x => bin(body_mass_g, maxbins => 20) OVER panel, y => count(*))
   GROUP BY island
|> ORDER BY island
```

### Species share per island

The share of each species on each island, from a window of the counts of each panel.

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => species,
     y => y(CAST(count(*) AS FLOAT64) / sum(count(*)) OVER panel, title => 'Share'),
   )
   GROUP BY island
|> ORDER BY island
```

### Sex donut per species

A donut of the sexes of each species.

```sql
FROM samples.penguins
|> AGGREGATE mark_polar_bar(
     theta => stack(count(*)),
     color => sex,
     coord => polar(inner_radius => 0.5),
   )
   GROUP BY species
|> ORDER BY species
```

### Two charts per group

Two charts for each species and island, the bills and the sizes, from an `AGGREGATE` with two mark functions.

```sql
FROM samples.penguins
|> AGGREGATE
     count(*) AS n,
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => sex) AS bills,
     mark_symbol(x => flipper_length_mm, y => body_mass_g, color => sex) AS sizes,
   GROUP BY species, island
|> ORDER BY species, island
```

### MPG vs. weight per origin

Miles per gallon against weight for each origin.

```sql
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => weight,
     y => mpg,
     color => color(CAST(cylinders AS STRING), title => 'Cylinders'),
   )
   GROUP BY origin
|> ORDER BY origin
```

### Mean MPG by cylinders

The mean miles per gallon of each model year for each origin, by number of cylinders.

```sql
FROM samples.cars
|> AGGREGATE mark_line(
     x => year,
     y => avg(mpg),
     color => color(CAST(cylinders AS STRING), title => 'Cylinders'),
   )
   GROUP BY origin
|> ORDER BY origin
```

### Genre ratings with stats

The ratings of the movies of each genre, with the number and the mean IMDB rating of its movies, from the genre with the most movies.

```sql
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE
     count(*) AS movies,
     round(avg(imdb_rating), 1) AS mean_imdb_rating,
     mark_symbol(
       x => x(rotten_tomatoes_rating, scale => scale(domain => [0, 100])),
       y => y(imdb_rating, scale => scale(domain => [0, 10])),
       opacity => 0.4,
     ) AS chart,
   GROUP BY genre
|> ORDER BY movies DESC
```

### IMDB histograms by genre

A histogram of the IMDB ratings of each genre, with the bins of all the movies.

```sql
FROM samples.movies
|> WHERE genre IS NOT NULL AND imdb_rating IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(imdb_rating, maxbins => 20), title => 'IMDB rating'),
     y => count(*),
   )
   GROUP BY genre
|> ORDER BY genre
```

### Sepals and petals

Sepals and petals for each species of iris.

```sql
FROM samples.iris
|> AGGREGATE
     mark_symbol(x => `SepalLengthCm`, y => `SepalWidthCm`) AS sepals,
     mark_symbol(x => `PetalLengthCm`, y => `PetalWidthCm`) AS petals,
   GROUP BY `Species`
|> ORDER BY `Species`
```

### Temperatures by year

The highest temperature of each day, a year to a row, grouped by an expression.

```sql
FROM samples.seattle_weather
|> AGGREGATE mark_line(x => date, y => temp_max) GROUP BY EXTRACT(YEAR FROM date) AS year
|> ORDER BY year
```

### Revenue and units per region

The revenue of each region, by month and stacked by product, and its units, by product and stacked by channel, from two mark calls of one `AGGREGATE`, with the total revenue of each region.

```sql
FROM samples.sales
|> AGGREGATE
     mark_area(x => month, y => stack(sum(revenue)), color => product) AS revenue,
     mark_bar(x => product, y => stack(sum(units)), color => channel) AS units,
     sum(revenue) AS total_revenue,
   GROUP BY region
|> ORDER BY total_revenue DESC
```

## Composition

A facet lays out the charts of a column by a key, with a label for each value. `facet_col` puts them in a row, `facet_row` in a column, and `facet_wrap` in rows that fill the width. An `AGGREGATE` of a mark call with `GROUP BY` gives the charts and their keys.

### Bill panels by island

Bill depth against bill length, a panel for each island.

```sql
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => species,
   ) AS chart
   GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart
```

### Grid of sex and island

A grid of panels, a row for each sex and a column for each island, from a `facet_col` for each sex in a `facet_row`.

```sql
FROM samples.penguins
|> WHERE sex IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => species,
   ) AS chart
   GROUP BY sex, island
|> AGGREGATE facet_column(chart, island) AS chart GROUP BY sex
|> AGGREGATE facet_row(chart, sex) AS chart
```

### Wrapped genre histograms

A histogram of the IMDB ratings of each genre, in rows of at most four.

```sql
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_bar(x => bin(imdb_rating), y => count(*)) AS chart GROUP BY genre
|> AGGREGATE facet_wrap(chart, genre, max_columns => 4) AS chart
```

### Independent histogram scales

The same histograms, with the bins of each genre and a y scale for each panel.

```sql
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_bar(x => bin(imdb_rating) OVER panel, y => count(*)) AS chart GROUP BY genre
|> AGGREGATE facet_wrap(
     chart,
     genre,
     max_columns => 4,
     resolve => resolve(y => 'independent'),
   ) AS chart
```

### Facet with a title

Miles per gallon against weight for each number of cylinders, with a facet title.

```sql
FROM samples.cars
|> AGGREGATE mark_symbol(x => weight, y => mpg, color => origin) AS chart GROUP BY cylinders
|> AGGREGATE facet_wrap(chart, cylinders, facet => facet(title => 'Cylinders')) AS chart
```

### Yearly rows, own x scales

The highest temperature of each day, a year to a row, with an x scale for each year.

```sql
FROM samples.seattle_weather
|> AGGREGATE mark_line(x => date, y => temp_max) AS chart GROUP BY EXTRACT(YEAR FROM date) AS year
|> AGGREGATE facet_row(
     chart,
     year,
     resolve => resolve(x => 'independent'),
   ) AS chart
```

### Wider gap between panels

The revenue of each region, stacked by product, with a wider gap between the panels.

```sql
FROM samples.sales
|> AGGREGATE mark_area(x => month, y => stack(sum(revenue)), color => product) AS chart
   GROUP BY region
|> AGGREGATE facet_column(chart, region, gap => 24) AS chart
```

### Bars with a mean rule

`layer` draws its charts in one plot, each over the ones before it.

Bars of the mean body mass of each species, with a rule at the mean of all the penguins.

```sql
FROM samples.penguins
|> AGGREGATE layer(
     mark_bar(x => species, y => y(avg(body_mass_g), title => 'Mean body mass (g)')),
     mark_rule(y => y(avg(body_mass_g), title => 'Mean body mass (g)'), stroke_width => 2),
   )
```

### Scatter with mean labels

The penguins, with a label at the mean bill of each species.

```sql
FROM samples.penguins
|> AGGREGATE layer(
     mark_symbol(
       x => x(
         bill_length_mm,
         scale => scale(zero => FALSE),
         title => 'Bill length (mm)',
       ),
       y => y(bill_depth_mm, scale => scale(zero => FALSE), title => 'Bill depth (mm)'),
       color => species,
       opacity => 0.4,
     ),
     mark_text(
       x => x(avg(bill_length_mm), title => 'Bill length (mm)'),
       y => y(avg(bill_depth_mm), title => 'Bill depth (mm)'),
       text => species,
       size => 14,
     ),
   )
```

### Range with a mean line

The range of temperatures of each month, with a line and points at the mean of the highest temperatures.

```sql
FROM samples.seattle_weather
|> EXTEND date_trunc(date, MONTH) AS month
|> AGGREGATE layer(
     mark_rule(
       x => month,
       y => y(min(temp_min), title => 'Temperature (°C)'),
       y2 => max(temp_max),
       opacity => 0.3,
       stroke_width => 6,
     ),
     mark_line(x => month, y => y(avg(temp_max), title => 'Temperature (°C)')),
     mark_symbol(x => month, y => y(avg(temp_max), title => 'Temperature (°C)'), size => 40),
   )
```

### Bills and sizes side by side

`hconcat` puts its charts side by side and `vconcat` puts them one above another, in the order of the arguments.

The bills and the sizes of the penguins, side by side.

```sql
FROM samples.penguins
|> AGGREGATE hconcat(
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species),
     mark_symbol(x => flipper_length_mm, y => body_mass_g, color => species),
   )
```

### Composed after the AGGREGATE

The same charts as named columns, composed after the `AGGREGATE`.

```sql
FROM samples.penguins
|> AGGREGATE
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species) AS bills,
     mark_symbol(x => flipper_length_mm, y => body_mass_g, color => species) AS sizes,
|> SELECT hconcat(bills, sizes) AS chart
```

### Weighted concat tracks

A scatter plot three times as wide as a bar chart of its species, from the tracks of an `hconcat`.

```sql
FROM samples.penguins
|> AGGREGATE hconcat(
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species),
     mark_bar(x => count(*), y => species, color => species),
     gap => 30,
     tracks => [track(share => 3), track(share => 1)],
   )
```

### Three charts in a column

The penguins by species, by island, and by sex, one above another, with a title.

```sql
FROM samples.penguins
|> AGGREGATE vconcat(
     mark_bar(x => species, y => count(*)),
     mark_bar(x => island, y => count(*)),
     mark_bar(x => sex, y => count(*)),
     title => 'Penguins',
   )
```

### hconcat in a facet_row

The bills and the counts of the species on each island, a row for each island, from an `hconcat` for each island in a `facet_row`.

```sql
FROM samples.penguins
|> AGGREGATE hconcat(
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species),
     mark_bar(x => species, y => count(*), color => species),
   ) AS chart
   GROUP BY island
|> AGGREGATE facet_row(chart, island) AS chart
```

### vconcat_agg by island

`layer_agg`, `hconcat_agg`, and `vconcat_agg` compose the charts of a group, in the order of an `ORDER BY` in the call. The species of each island, one above another, without the labels of a facet.

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(x => species, y => count(*), color => species) AS chart GROUP BY island
|> AGGREGATE vconcat_agg(chart ORDER BY island) AS chart
```

## Math

A title, a subtitle, the label markup of an axis or a legend, and a `mark_text` with `syntax => 'typst'` read Typst markup, so they can hold math. `display(...)` lays out a fraction, a root, a sum, or an integral at its full size inside a label, and `\` breaks the lines of a text mark. Each formula here is the subtitle of its chart, and each value that SQL computes is a `mark_text` at a direct position in the plot, which `layer` composes with the chart. Labels do not support block equations, matrices, vectors, `cases`, alignment, or statements such as `#let`.

### Normal fit to body mass

A histogram of the body mass of the penguins of one species with the normal density of their mean and standard deviation, scaled to the counts. The variable `chosen` names the species, and `bin_width` gives both the width of the bins and the scale of the density, so the curve fits the bars at any width. The density formula is the subtitle, and the label in the plot is Typst markup that SQL builds from the fitted values.

```sql
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
```

### Normal CDF as an area

The standard normal density, shaded up to 1.96, with the integral of the cumulative distribution. A sum over the generated rows gives the shaded area, which the label beside the bound reads.

```sql
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
```

### Least-squares line

Miles per gallon against weight with the least-squares line of `regr_intercept` and `regr_slope`, the estimator of the slope as the subtitle, and the fitted line and its R² as markup that SQL builds.

```sql
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
```

## Pan, zoom, and tooltips

The `tools` option of a mark call declares its tools, and a `layer` declares the tools that its marks share: `navigate()` pans with a drag and zooms with the wheel, and `tooltip()` shows the row of the mark under the pointer. A single chart is interactive in the result pane. In a table, check "Interactive charts", or double-click a chart to expand it. A tooltip shows the columns of the chart's marks, so a column appears in it only when a channel encodes it, and each row takes the title of its channel, such as `count(*)`.

### Pan and zoom

Miles per gallon against weight, with pan and zoom.

```sql
FROM samples.cars
|> AGGREGATE mark_symbol(x => weight, y => mpg, color => origin, tools => [navigate()])
```

### Nearest-point tooltip

The same scatter plot, with a tooltip of the car nearest the pointer, within as many pixels as the variable `radius`.

```sql
DECLARE radius FLOAT64 DEFAULT 20;
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => weight,
     y => mpg,
     color => origin,
     tools => [navigate(), tooltip(nearest => TRUE, max_radius => radius)],
   )
```

### Tooltip titles and fields

A tooltip with the origin as its title and fields with titles of their own.

```sql
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => weight,
     y => mpg,
     color => origin,
     size => cylinders,
     tools => [
       tooltip(
         nearest => TRUE,
         max_radius => 20,
         title => 'origin',
         fields => [
           field(column => 'weight', title => 'Weight (lb)'),
           field(column => 'mpg', title => 'Miles per gallon'),
           field(column => 'cylinders', title => 'Cylinders'),
         ],
       ),
     ],
   )
```

### Tooltip following the pointer

A tooltip that follows the pointer, without the swatch of the mark's color.

```sql
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => weight,
     y => mpg,
     color => origin,
     tools => [tooltip(anchor => 'pointer', hide_swatch => TRUE)],
   )
```

### Pan and zoom along x

The highest temperature of each day, with pan and zoom along x only, and a tooltip of the day nearest the pointer along x.

```sql
FROM samples.seattle_weather
|> AGGREGATE mark_line(
     x => date,
     y => temp_max,
     tools => [navigate(channels => ['x']), tooltip(nearest => TRUE, metric => 'along_x')],
   )
```

### Pan and zoom within limits

Pan and zoom that stay inside the data, with a smallest span of 200 lb and 2 miles per gallon.

```sql
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => x(
       weight,
       scale => scale(navigation => navigation('data', min_span => 200)),
     ),
     y => y(
       mpg,
       scale => scale(navigation => navigation('data', min_span => 2)),
     ),
     color => origin,
     tools => [navigate()],
   )
```

### Stacked bar tooltips

Stacked bars with a tooltip of each segment.

```sql
FROM samples.penguins
|> AGGREGATE mark_bar(x => species, y => stack(count(*)), color => island, tools => [tooltip()])
```

### Histogram bin tooltips

A histogram with a tooltip of each bin.

```sql
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(body_mass_g, maxbins => 20), title => 'Body mass (g)'),
     y => y(count(*), title => 'Penguins'),
     tools => [navigate(channels => ['x']), tooltip()],
   )
```

### Interactive genre ratings

The ratings of the movies, with the four largest genres and the rest as Other, with pan, zoom, and a tooltip.

```sql
FROM samples.movies
|> WHERE genre IS NOT NULL
|> AGGREGATE mark_symbol(
     x => rotten_tomatoes_rating,
     y => imdb_rating,
     color => lump(genre, n => 4),
     opacity => 0.4,
     tools => [navigate(), tooltip(nearest => TRUE, max_radius => 10)],
   )
```

### Facet with shared panning

A facet whose panels pan, zoom, and show tooltips. The panels share their scales, so a pan in one moves them all.

```sql
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => species,
     tools => [navigate(), tooltip()],
   ) AS chart
   GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart
```

### Panels that pan apart

The same panels with an x scale for each, so each pans on its own.

```sql
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => species,
     tools => [navigate(), tooltip()],
   ) AS chart
   GROUP BY island
|> AGGREGATE facet_column(
     chart,
     island,
     resolve => resolve(x => 'independent'),
   ) AS chart
```

### Layer with shared tools

A layer whose plot pans, zooms, and shows the tooltips of both of its layers, from the `tools` of `layer`.

```sql
FROM samples.penguins
|> AGGREGATE layer(
     mark_bar(x => species, y => y(avg(body_mass_g), title => 'Mean body mass (g)')),
     mark_rule(y => y(avg(body_mass_g), title => 'Mean body mass (g)'), stroke_width => 2),
     tools => [navigate(), tooltip()],
   )
```

### Table of interactive charts

A table of charts with tools, one for each species. Check "Interactive charts" to pan, zoom, and see the tooltips in place.

```sql
FROM samples.penguins
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => sex,
     tools => [navigate(), tooltip()],
   )
   GROUP BY species
|> ORDER BY species
```
