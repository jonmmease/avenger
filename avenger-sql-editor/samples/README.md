# Sample data

The editor embeds these files and registers each as a table of the schema `samples`, such as `samples.penguins`. The example queries of `../examples.md` read them.

| File | Rows | Source | Terms |
|---|---|---|---|
| `penguins.csv` | 344 | The Palmer Archipelago penguins of [palmerpenguins](https://github.com/allisonhorst/palmerpenguins) (Horst, Hill, and Gorman), from the Palmer Station Antarctica LTER, as seaborn-data ships them | CC0 1.0 |
| `cars.csv` | 406 | The cars of the [vega-datasets](https://github.com/vega/vega-datasets) `cars.json`, from the StatLib Datasets Archive, with snake_case column names and the model year as a date | Not specified. StatLib distributed the original in 1982 for educational and scientific purposes. |
| `movies.csv` | 2,259 | The movies of the vega-datasets `movies.json` with a title, both ratings, a major genre, and a release year, as `examples/movie-search` holds them | vega-datasets states no source or license for this file |
| `iris.csv` | 150 | Fisher's iris data (1936), as the [UCI Machine Learning Repository](https://archive.ics.uci.edu/dataset/53/iris) distributes it, in the layout of the Kaggle "Iris Species" file | CC BY 4.0 |
| `seattle_weather.csv` | 1,461 | The daily weather of Seattle from 2012 through 2015 of the vega-datasets `seattle-weather.csv`, from the NOAA National Climatic Data Center | A work of the U.S. government, in the public domain |
| `sales.csv` | 203 | Illustrative monthly revenue and units of three products in three regions through two channels in 2025, generated with a fixed seed, with about one month in twelve missing for each combination | Ours |
