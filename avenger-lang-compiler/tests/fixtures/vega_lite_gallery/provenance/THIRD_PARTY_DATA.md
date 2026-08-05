# Third-Party Vega Gallery Data

This inventory is generated from the pinned Vega datasets 3.2.1 Data Package.
The repository-level BSD-3-Clause license covers package code and infrastructure,
not every dataset. The upstream metadata is a reference starting point and does
not guarantee that a particular downstream use is permitted.

## `7zip.png`

Application icon from open-source software project. Used in [Image-based Scatter Plot example](https://vega.github.io/vega-lite/examples/scatter_image.html).

- Data Package name: `icon_7zip`
- Format: `png`
- Git-blob hash: `sha1:6586d6c00887cd48850099c174a42bb1677ade0c`
- Licenses: [GNU Lesser General Public License](https://www.7-zip.org/license.txt)
- Sources: [7-Zip](https://www.7-zip.org/)

## `airports.csv`

Airports in the United States and its territories, including major commercial, regional, and municipal airports. Contains information about each airport's location (latitude/longitude coordinates), identification codes, name, city, state, and country. While the exact generation source of this file is unknown, this data is consistent with files provided on a monthly frequency by the FAA's [National Airspace System Resource](https://www.faa.gov/air_traffic/flight_info/aeronav/aero_data/NASR_Subscription/).

- Data Package name: `airports`
- Format: `csv`
- Git-blob hash: `sha1:114c202bcc6784c8358b54f9ef152c54ec6c5fba`
- Licenses: https://www.usa.gov/government-works
- Sources: [Federal Aviation Administration](https://www.faa.gov/air_traffic/flight_info/aeronav/aero_data/NASR_Subscription/)

## `anscombe.json`

Eleven (x,y) pairs of numbers, with means x̄=9.0 and ȳ=7.5, and identical linear regression lines (same slope and intercept) and correlation coefficients (approximately 0.816). When plotted, reveals starkly different patterns: one shows a linear relationship, another a non-linear curve, the third a near-perfect linear relationship disrupted by a single outlier, and the fourth a near-vertical line of points where a single outlier entirely dictates the regression. In his 1973 paper "Graphs in Statistical Analysis" Yale Professor [Francis Anscombe](https://archives.yale.edu/repositories/12/resources/3711) uses these four datasets to argue that visualization is essential to good statistical work, not merely an optional supplement. This was a radical position at a time when most statistical analysis was done through batch processing on mainframes with no graphical output. Serves as a powerful demonstration that identical summary statistics can mask radically different patterns in data, making the case that statistical analysis should combine both numerical calculations and graphical examination.

- Data Package name: `anscombe`
- Format: `json`
- Git-blob hash: `sha1:11ae97090b6263bdf0c8661156a44a5b782e0787`
- Licenses: not specified in the pinned Data Package
- Sources: [Anscombe's quartet (Wikipedia)](https://en.wikipedia.org/wiki/Anscombe%27s_quartet#Data), [Anscombe, F. J. (1973). Graphs in Statistical Analysis. The American Statistician, 27(1):17-21.](https://www.jstor.org/stable/2682899)

## `barley.json`

Yields of barley varieties from experiments conducted by the Minnesota Agricultural Experiment Station (MAES) across six sites in Minnesota. The USDA Technical Bulletin No. 735 (December 1940) republished these yields data with explicit credit to MAES as the source. It was analyzed by agronomists F.R. Immer, H.K. Hayes, and L. Powers in the 1934 paper "Statistical Determination of Barley Varietal Adaption". R.A. Fisher popularized its use in the field of statistics when he included it in his book "The Design of Experiments". Since then it has been used to demonstrate new visualization techniques, including the trellis charts developed by Richard Becker, William Cleveland and others in the 1990s.

- Data Package name: `barley`
- Format: `json`
- Git-blob hash: `sha1:8dc50de2509b6e197ce95c24c98f90d9d1ab138c`
- Licenses: Dataset collected by Minnesota Agricultural Experiment Station - license status unspecified
- Sources: [The Design of Experiments Reference](https://en.wikipedia.org/wiki/The_Design_of_Experiments), [Wiebe, G. A., Reinbach-Welch, L., Cowan, P. R. (1940). Yields of Barley Varieties in the United States and Canada, 1932-36. United States: U.S. Department of Agriculture.](https://books.google.com/books?id=OUfxLocnpKkC&pg=PA19)

## `cars.json`

Collection of car specifications and performance metrics from various automobile manufacturers.

- Data Package name: `cars`
- Format: `json`
- Git-blob hash: `sha1:1d56d3fa6da01af9ece2d6397892fe5bb6f47c3d`
- Licenses: [The original was distributed in 1982 for educational and scientific purposes.](http://lib.stat.cmu.edu/datasets/cars.desc)
- Sources: [StatLib Datasets Archive](http://lib.stat.cmu.edu/datasets/)

## `co2-concentration.csv`

Atmospheric CO2 concentration measurements from Mauna Loa Observatory, Hawaii. Contains monthly readings from 1958-2020 with two key measurements: 1. CO2 concentrations in millionths of a [mole](https://en.wikipedia.org/wiki/Mole_(unit)) of CO2 per mole of air (parts per million), reported on the 2012 SIO manometric mole fraction scale 2. Seasonally adjusted values where a [4-harmonic fit](https://en.wikipedia.org/wiki/Harmonic_analysis) with linear gain factor has been subtracted to remove the quasi-regular seasonal cycle Values are adjusted to 24:00 hours on the 15th of each month. Only includes rows with valid data.

- Data Package name: `co2_concentration`
- Format: `csv`
- Git-blob hash: `sha1:b8715cbd2a8d0c139020a73fdb4d231f8bde193a`
- Licenses: [Creative Commons Attribution 4.0](https://creativecommons.org/licenses/by/4.0/)
- Sources: [Scripps CO2 Program](https://scrippsco2.ucsd.edu/data/atmospheric_co2/primary_mlo_co2_record), [In-situ CO2 Data](https://scrippsco2.ucsd.edu/assets/data/atmospheric/stations/in_situ_co2/monthly/monthly_in_situ_co2_mlo.csv)

## `countries.json`

Key demographic indicators (life expectancy at birth and fertility rate measured as babies per woman) for various countries from 1955 to 2000 at 5-year intervals. Includes both current values and adjacent time period values (previous and next) for each indicator. Gapminder's [data documentation](https://www.gapminder.org/data/documentation/) notes that its philosophy is to fill data gaps with estimates and use current geographic boundaries for historical data. Gapminder states that it aims to "show people the big picture" rather than support detailed numeric analysis.

- Data Package name: `countries`
- Format: `json`
- Git-blob hash: `sha1:0070959b7f1a09475baa5099098240ae81026e72`
- Licenses: [Creative Commons Attribution 4.0 International](https://www.gapminder.org/free-material/)
- Sources: [Gapminder Foundation - Life Expectancy](https://docs.google.com/spreadsheets/d/1RehxZjXd7_rG8v2pJYV6aY0J3LAsgUPDQnbY4dRdiSs/edit?gid=176703676#gid=176703676), [Gapminder Foundation - Fertility](https://docs.google.com/spreadsheets/d/1aLtIpAWvDGGa9k2XXEz6hZugWn0wCd5nmzaRPPjbYNA/edit?gid=176703676#gid=176703676)

## `disasters.csv`

Annual number of deaths from disasters, sourced from EM-DAT (Emergency Events Database) maintained by the Centre for Research on the Epidemiology of Disasters (CRED) at UCLouvain, Belgium. Processed by Our World in Data to standardize country names and world region definitions, converting units, calculating derived indicators, and adapting metadata. Deaths are reported as absolute numbers.

- Data Package name: `disasters`
- Format: `csv`
- Git-blob hash: `sha1:0584ed86190870b0089d9ea67c94f3dd3feb0ec8`
- Licenses: [EM-DAT terms of use](https://doc.emdat.be/docs/legal/terms-of-use/), [Creative Commons BY license (Our World in Data)](https://creativecommons.org/licenses/by/4.0/)
- Sources: [EM-DAT: The Emergency Events Database](https://www.emdat.be), [Hannah Ritchie, Pablo Rosado and Max Roser (2022) - Natural Disasters](https://ourworldindata.org/natural-catastrophes)

## `driving.json`

Tracks the relationship between driving habits and gasoline prices in the United States during a period spanning multiple significant events, including the cheap gas era, Arab oil embargo, energy crisis, record low prices, and the "swing backward" from 1956 to 2010.

- Data Package name: `driving`
- Format: `json`
- Git-blob hash: `sha1:33d0afc57fb1005e69cd3e8a6c77a26670d91979`
- Licenses: not specified in the pinned Data Package
- Sources: [New York Times (citing U.S. Energy Information Administration, Federal Highway Administration, and Brookings Institution)](https://archive.nytimes.com/www.nytimes.com/imagepages/2010/05/02/business/02metrics.html)

## `earthquakes.json`

Represents approximately one week of continuous monitoring from USGS's "all earthquakes" real-time feed, which includes 1,703 seismic events of all magnitudes recorded by the USGS Earthquake Hazards Program from January 31 to February 7, 2018 (UTC).

- Data Package name: `earthquakes`
- Format: `geojson`
- Git-blob hash: `sha1:ed4c47436c09d5cc5f428c233fbd8074c0346fd0`
- Licenses: [U.S. Public Domain](https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits)
- Sources: [USGS Earthquake Feed](https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/all_week.geojson)

## `ffox.png`

Application icon from open-source software project. Used in [Image-based Scatter Plot example](https://vega.github.io/vega-lite/examples/scatter_image.html).

- Data Package name: `ffox`
- Format: `png`
- Git-blob hash: `sha1:0691709484a75e9d8ee55a22b1980d67d239c2c4`
- Licenses: [Mozilla Trademark License](https://www.mozilla.org/en-US/foundation/trademarks/policy/)
- Sources: [Mozilla Firefox](https://www.mozilla.org/firefox/)

## `flights-2k.json`

Flight delay statistics (2,000 rows) from U.S. Bureau of Transportation Statistics. Collected under regulatory reporting requirements (14 CFR Part 234), which mandate that qualifying airlines report on-time performance data to BTS. Transformed using `/scripts/flights.py`

- Data Package name: `flights_2k`
- Format: `json`
- Git-blob hash: `sha1:d9221dc7cd477209bf87e680be3c881d8fee53cd`
- Licenses: [Data Collected Under U.S. DOT Regulatory Requirements - License Terms Not Explicitly Specified](https://www.ecfr.gov/current/title-14/chapter-II/subchapter-A/part-234)
- Sources: [U.S. Bureau of Transportation Statistics](https://www.transtats.bts.gov/DL_SelectFields.asp?gnoyr_VQ=FGJ&QO_fu146_anzr=b0-gvzr)

## `flights-5k.json`

Flight delay statistics (5,000 rows) from U.S. Bureau of Transportation Statistics. Collected under regulatory reporting requirements (14 CFR Part 234), which mandate that qualifying airlines report on-time performance data to BTS. Transformed using `/scripts/flights.py`

- Data Package name: `flights_5k`
- Format: `json`
- Git-blob hash: `sha1:8459fa09e3ba8197928b5dba0b9f5cc380629758`
- Licenses: [Data Collected Under U.S. DOT Regulatory Requirements - License Terms Not Explicitly Specified](https://www.ecfr.gov/current/title-14/chapter-II/subchapter-A/part-234)
- Sources: [U.S. Bureau of Transportation Statistics](https://www.transtats.bts.gov/DL_SelectFields.asp?gnoyr_VQ=FGJ&QO_fu146_anzr=b0-gvzr)

## `flights-airport.csv`

Flight information for the year 2008. Each record consists of an origin airport (identified by IATA id), a destination airport, and the count of flights along this route.

- Data Package name: `flights_airport`
- Format: `csv`
- Git-blob hash: `sha1:0ba03114891e97cfc3f83d9e3569259e7f07af7b`
- Licenses: [U.S. Government Dataset](https://www.usa.gov/government-works)
- Sources: [U.S. Bureau of Transportation Statistics](https://www.transtats.bts.gov/DL_SelectFields.asp?gnoyr_VQ=FGJ&QO_fu146_anzr=b0-gvzr)

## `gapminder-health-income.csv`

Per-capita income, life expectancy, population and regional grouping. Reference year for the data is not specified. Gapminder historical data is subject to revisions. Gapminder (v30, 2023) defines per-capita income as follows: >"This is real GDP per capita (gross domestic product per person adjusted for inflation) >converted to international dollars using purchasing power parity rates. An international dollar >has the same purchasing power over GDP as the U.S. dollar has in the United States."

- Data Package name: `gapminder_health_income`
- Format: `csv`
- Git-blob hash: `sha1:abce37a932917085023a345b1a004396e9355ac3`
- Licenses: [Creative Commons Attribution 4.0 International](https://www.gapminder.org/free-material/)
- Sources: [Gapminder Foundation](https://www.gapminder.org), [Gapminder GDP Per Capita Data](https://docs.google.com/spreadsheets/d/1i5AEui3WZNZqh7MQ4AKkJuCz4rRxGR_pw_9gtbcBOqQ/edit?gid=501532268#gid=501532268)

## `gapminder.json`

Combines key demographic indicators (life expectancy at birth, population, and fertility rate measured as babies per woman) for various countries from 1955 to 2005 at 5-year intervals. Includes a 'cluster' column, a categorical variable grouping countries. Gapminder's data documentation notes that its philosophy is to fill data gaps with estimates and use current geographic boundaries for historical data. Gapminder states that it aims to "show people the big picture" rather than support detailed numeric analysis. Notes: 1. Country Selection: The set of countries matches the version of this dataset originally added to this collection in 2015. The specific criteria for country selection in that version are not known. Data for Aruba are no longer available in the new version. Hong Kong has been revised to Hong Kong, China in the new version. 2. Data Precision: The precision of float values may have changed from the original version. These changes reflect the most recent source data used for each indicator. 3. Regional Groupings: To preserve continuity with previous versions of this dataset, we have retained the column name 'cluster' instead of renaming it to 'six_regions'.

- Data Package name: `gapminder`
- Format: `json`
- Git-blob hash: `sha1:8cb2f0fc23ce612e5f0c7bbe3dcac57f6764b7b3`
- Licenses: [Creative Commons Attribution 4.0 International](https://www.gapminder.org/free-material/)
- Sources: [Gapminder Foundation - Life Expectancy (Data)](https://docs.google.com/spreadsheets/d/1RehxZjXd7_rG8v2pJYV6aY0J3LAsgUPDQnbY4dRdiSs/edit?gid=176703676#gid=176703676), [Gapminder Foundation - Life Expectancy (Documentation)](https://www.gapminder.org/data/documentation/gd004/), [Gapminder Foundation - Population (Data)](https://docs.google.com/spreadsheets/d/1c1luQNdpH90tNbMIeU7jD__59wQ0bdIGRFpbMm8ZBTk/edit?gid=176703676#gid=176703676), [Gapminder Foundation - Population (Documentation)](https://www.gapminder.org/data/documentation/gd003/), [Gapminder Foundation - Fertility (Data)](https://docs.google.com/spreadsheets/d/1aLtIpAWvDGGa9k2XXEz6hZugWn0wCd5nmzaRPPjbYNA/edit?gid=176703676#gid=176703676), [Gapminder Foundation - Fertility Documentation (Documentation)](https://www.gapminder.org/data/documentation/gd008/), [Gapminder Foundation - Data Geographies (Data)](https://docs.google.com/spreadsheets/d/1qHalit8sXC0R8oVXibc2wa2gY7bkwGzOybEMTWp-08o/edit?gid=1597424158#gid=1597424158), [Gapminder Foundation - Data Geographies (Documentation)](https://www.gapminder.org/data/geo/), [Gapminder Data Documentation](https://www.gapminder.org/data/documentation/)

## `gimp.png`

Application icon from open-source software project. Used in [Image-based Scatter Plot example](https://vega.github.io/vega-lite/examples/scatter_image.html).

- Data Package name: `gimp`
- Format: `png`
- Git-blob hash: `sha1:cf0505dd72eb52558f6f71bd6f43663df4f2f82c`
- Licenses: [notspecified](https://www.gimp.org/docs/userfaq.html#whats-the-gimps-license-and-how-do-i-comply-with-it)
- Sources: [GIMP - About GIMP](https://www.gimp.org/about/)

## `github.csv`

Simulated GitHub contribution data showing hourly commit counts across different times of day. Designed to demonstrate typical patterns of developer activity in a GitHub-style punchcard visualization format.

- Data Package name: `github`
- Format: `csv`
- Git-blob hash: `sha1:18547064dd687c328ea2fb5023cae6417ca6f050`
- Licenses: [BSD-3-Clause](https://github.com/vega/vega-datasets/blob/main/scripts/LICENSE)
- Sources: [Generated using `/scripts/github.py`.](https://github.com/vega/vega-datasets/blob/main/scripts/github.py)

## `income.json`

Household income distribution by US state, derived from the Census Bureau's American Community Survey 3-Year Data (2013). The dataset shows the percentage of households within different income brackets for each state. Generated using `/scripts/income.py`. This product uses the Census Bureau Data API but is not endorsed or certified by the Census Bureau.

- Data Package name: `income`
- Format: `json`
- Git-blob hash: `sha1:50bc780ef4a81e4f67c5ab2686ff10ba9798a951`
- Licenses: [U.S. Census Bureau API Terms of Service](https://www.census.gov/data/developers/about/terms-of-service.html)
- Sources: [U.S. Census Bureau American Community Survey 3-Year Data (2013)](https://www.census.gov/data/developers/data-sets/acs-3year/2013.html), [Census Bureau Data API User Guide](https://www.census.gov/data/developers/guidance/api-user-guide.html)

## `londonBoroughs.json`

Boundaries of London boroughs reprojected and simplified from `London_Borough_Excluding_MHW` shapefile. Original data "contains National Statistics data © Crown copyright and database right (2015)" and "Contains Ordnance Survey data © Crown copyright and database right [2015].

- Data Package name: `london_boroughs`
- Format: `topojson`
- Git-blob hash: `sha1:d90805055ffdfe5163a7655c4847dc61df45f92b`
- Licenses: [UK Open Government License](https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/)
- Sources: [Statistical GIS Boundary Files, London Datastore](https://data.london.gov.uk/dataset/statistical-gis-boundary-files-for-london-20od9/)

## `londonCentroids.json`

Calculated from `londonBoroughs.json` using [`d3.geoCentroid`](https://d3js.org/d3-geo/math#geoCentroid).

- Data Package name: `london_centroids`
- Format: `json`
- Git-blob hash: `sha1:2e24c01140cfbcad5e1c859be6df4efebca2fbf5`
- Licenses: [UK Open Government License](https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/)
- Sources: [[londonBoroughs.json](https://github.com/vega/vega-datasets/blob/main/data/londonBoroughs.json) from the [vega-datasets](https://github.com/vega/vega-datasets) repository](https://github.com/vega/vega-datasets/blob/main/data/londonBoroughs.json)

## `londonTubeLines.json`

A [topologically-encoded](https://github.com/topojson/topojson) representation of select London Underground rail lines, derived from OpenStreetMap data. These 394 LineString geometries, encoded using 406 arcs, depict transport paths between stations with stations marked as nodes along the lines. Originally transformed from a GeoJSON intermediary `tfl_lines.json` into TopoJSON format, this network configuration reflects the system as of February 4, 2018, and may not incorporate subsequent modifications or expansions.

- Data Package name: `london_tube_lines`
- Format: `topojson`
- Git-blob hash: `sha1:1b21ea5339320090b106082bd9d39a1055aadb18`
- Licenses: [Open Data Commons Open Database License (ODbL)](https://opendatacommons.org/licenses/odbl/)
- Sources: [OpenStreetMap Data (processed by oobrien/vis)](https://github.com/oobrien/vis/blob/master/tubecreature/data/tfl_lines.json)

## `lookup_groups.csv`

A nine-row lookup table for the `lookup_people.csv` dataset, mapping people to groups. Used to [demonstrate](https://vega.github.io/vega-lite/examples/lookup.html) `lookup` transforms.

- Data Package name: `lookup_groups`
- Format: `csv`
- Git-blob hash: `sha1:741df36729a9d84d18ec42f23a386b53e7e3c428`
- Licenses: [BSD-3-Clause](https://github.com/vega/vega-datasets/blob/main/scripts/LICENSE)
- Sources: Generated Data

## `lookup_people.csv`

A synthetic list of nine people and their associated name, age, and height in centimeters. Used in conjunction with `lookup_groups.csv` to [demonstrate](https://vega.github.io/vega-lite/examples/lookup.html) `lookup` transforms.

- Data Package name: `lookup_people`
- Format: `csv`
- Git-blob hash: `sha1:c79f69afb3ff81a0c8ddc01f5cf2f078e288457c`
- Licenses: [BSD-3-Clause](https://github.com/vega/vega-datasets/blob/main/scripts/LICENSE)
- Sources: Generated Data

## `monarchs.json`

A chronological list of English and British monarchs from Elizabeth I through George IV. Contains two intentional inaccuracies to maintain compatibility with the [Wheat and Wages](https://vega.github.io/vega/examples/wheat-and-wages/) example visualization: 1. the start date for the reign of Elizabeth I is shown as 1565, instead of 1558; 2. the end date for the reign of George IV is shown as 1820, instead of 1830. These discrepancies align the `monarchs.json` dataset with the start and end dates of the `wheat.json` dataset used in the visualization. The entry "W&M" represents the joint reign of William III and Mary II. While the dataset shows their reign as 1689-1702, the official Web site of the British royal family indicates that Mary II's reign ended in 1694, though William III continued to rule until 1702. The `commonwealth` field is used to flag the period from 1649 to 1660, which includes the Commonwealth of England, the Protectorate, and the period leading to the Restoration. While historically more accurate to call this the "interregnum," the field name of `commonwealth` from the original dataset is retained for backwards compatibility. > [!IMPORTANT] > Revised in Aug. 2024 to show James II's reign now ends in 1688 (previously 1689). Source data has been verified against the kings & queens and interregnum pages of the official website of the British royal family (retrieved in Aug. 2024).

- Data Package name: `monarchs`
- Format: `json`
- Git-blob hash: `sha1:921dfa487a4198cfe78f743aa0aa87ad921642df`
- Licenses: [Open Government Licence v3.0 (UK)](https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/)
- Sources: [The Royal Family - Kings & Queens](https://www.royal.uk/kings-and-queens-1066), [The Royal Family - Interregnum](https://www.royal.uk/interregnum-1649-1660)

## `movies.json`

A collection of films and their performance metrics, including box office earnings, budgets, and audience ratings. Contains known data quality issues typical of real-world datasets: - Some movie titles with numeric names (1776, 2012, 300, etc.) are stored as JSON numbers rather than strings - Release dates use 'MMM DD YYYY' format rather than ISO 8601 These characteristics make it suitable as a teaching resource for developing data cleaning and validation skills in real-world analysis workflows.

- Data Package name: `movies`
- Format: `json`
- Git-blob hash: `sha1:e38178f99454568c5160fc759184a1a1471cc558`
- Licenses: not specified in the pinned Data Package
- Sources: not specified in the pinned Data Package

## `normal-2d.json`

Five hundred paired coordinates sampled from a bivariate normal distribution. The data is centered near the origin with standard deviations indicating a relatively equal spread in both dimensions. The variables exhibit negligible correlation (0.026), suggesting independence. [Normality tests](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.normaltest.html) for each variable yield high p-values, supporting the normal distribution assumption. These characteristics make it well-suited for demonstrating statistical visualization techniques in Vega and Vega-Lite, including scatter plots, density plots, heatmaps, and marginal histograms/density curves. It can also serve as a clean baseline for testing new visualization methods or for educational purposes in data visualization and statistics. A contrast to uniformly distributed data in `uniform-2d.json`

- Data Package name: `normal_2d`
- Format: `json`
- Git-blob hash: `sha1:4303306ec275209fcba008cbd3a5f29c9e612424`
- Licenses: [BSD-3-Clause](https://github.com/vega/vega-datasets/blob/main/scripts/LICENSE)
- Sources: Generated Data

## `ohlc.json`

Performance of the Chicago Board Options Exchange [Volatility Index](https://en.wikipedia.org/wiki/VIX) (VIX) in the summer of 2009. The precise methodology used to derive the signal and calculate the ret columns is unclear.

- Data Package name: `ohlc`
- Format: `json`
- Git-blob hash: `sha1:9b3d93e8479d3ddeee29b5e22909132346ac0a3b`
- Licenses: not specified in the pinned Data Package
- Sources: [Yahoo Finance VIX Data](https://finance.yahoo.com/chart/%5EVIX), [CBOE - VIX Historical Data](https://www.cboe.com/tradable_products/vix/vix_historical_data/)

## `penguins.json`

Records of morphological measurements and demographic information from 344 Palmer Archipelago penguins across three species. Collected by [Dr. Kristen Gorman](https://www.uaf.edu/cfos/people/faculty/detail/kristen-gorman.php) and the Palmer Station Antarctica [LTER](https://lternet.edu/). Data gathering occurred as part of Palmer Station's long-term ecological research, contributing to studies of Antarctic marine ecosystems and penguin biology. All measurements follow standardized units, enabling research into morphological variations between species and sexual dimorphism in Antarctic penguins.

- Data Package name: `penguins`
- Format: `json`
- Git-blob hash: `sha1:517b6d3267174b1b65691a37cbd59c1739155866`
- Licenses: [Creative Commons Zero 1.0 Universal](https://github.com/allisonhorst/palmerpenguins?tab=CC0-1.0-1-ov-file#readme)
- Sources: [Palmer Station Antarctica LTER](https://pallter.marine.rutgers.edu/), [Allison Horst's Penguins Repository](https://github.com/allisonhorst/penguins)

## `population.json`

U.S. population counts by age group (0-90+ in 5-year intervals) and sex for each decade between 1850 and 2000, collected and harmonized from historical census records by IPUMS USA. IPUMS updates and revises datasets over time, which may result in discrepancies with current IPUMS data. When using this dataset, please refer to IPUMS USA terms of use. The organization requests the use of the following citation for this json file: Steven Ruggles, Katie Genadek, Ronald Goeken, Josiah Grover, and Matthew Sobek. Integrated Public Use Microdata Series: Version 6.0. Minneapolis: University of Minnesota, 2015. http://doi.org/10.18128/D010.V6.0

- Data Package name: `population`
- Format: `json`
- Git-blob hash: `sha1:680fd336e777314198450721c31227a11f02411f`
- Licenses: [IPUMS Terms of Use](https://www.ipums.org/about/terms)
- Sources: [IPUMS USA](https://usa.ipums.org/usa/)

## `population_engineers_hurricanes.csv`

Per-state population (2016 ACS 1-Year), ratio of engineers to total civilian employed population (2016 ACS 1-Year), and total hurricane landfalls (possibly 1851-2015). Used in Vega-Lite example, [Three Choropleths Representing Disjoint Data from the Same Table](https://vega.github.io/vega-lite/examples/geo_repeat.html)

- Data Package name: `population_engineers_hurricanes`
- Format: `csv`
- Git-blob hash: `sha1:3bad66ef911b93c641edc21f2034302348bffaf9`
- Licenses: [U.S. Government Dataset](https://www.usa.gov/government-works)
- Sources: [U.S. Census Bureau, 2016 ACS 1-Year Estimates: Total Population (B01001) and Occupation (S2401)](https://www.census.gov/data/developers/data-sets/acs-1year/2016.html), [Continental United States Hurricane Impacts/Landfalls](https://www.aoml.noaa.gov/hrd/hurdat/All_U.S._Hurricanes.html), [NOAA FAQ: How Many Landfalling Hurricanes Have Hit Eact State?](https://www.aoml.noaa.gov/hrd-faq/#landfalls-by-state)

## `seattle-weather-hourly-normals.csv`

Hourly weather normals with metric units. The 1981-2010 Climate Normals are NCDC's three-decade averages of climatological variables, including temperature and precipitation. Learn more in the [documentation](https://www1.ncdc.noaa.gov/pub/data/cdo/documentation/NORMAL_HLY_documentation.pdf). We only included temperature, wind, and pressure and updated the format to be easier to parse.

- Data Package name: `seattle_weather_hourly_normals`
- Format: `csv`
- Git-blob hash: `sha1:d55461adc9742bb061f6072b694aaf73e8b529db`
- Licenses: [U.S. Government Dataset](https://www.usa.gov/government-works)
- Sources: [NOAA National Climatic Data Center (NCDC)](https://www.ncdc.noaa.gov/cdo-web/datatools/normals)

## `seattle-weather.csv`

Daily weather in metric units. Transformed using `/scripts/weather.py`. The categorical "weather" field is synthesized from multiple fields in the original dataset. This data is intended for instructional purposes.

- Data Package name: `seattle_weather`
- Format: `csv`
- Git-blob hash: `sha1:0f38b53bdc1c42c5e5d484f33b9d4d7b229e0e59`
- Licenses: [U.S. Government Dataset](https://www.usa.gov/government-works)
- Sources: [NOAA National Climatic Data Center](https://www.ncdc.noaa.gov/cdo-web/datatools/records)

## `sp500.csv`

Monthly closing values of the S&P 500 stock market index from January 2000 to March 2010. Captures several significant market events including the dot-com bubble burst (2000-2002), the mid-2000s bull market, and the 2008 financial crisis.

- Data Package name: `sp500`
- Format: `csv`
- Git-blob hash: `sha1:0eb287fb7c207f4ed392821d67a92267180fc8cf`
- Licenses: not specified in the pinned Data Package
- Sources: not specified in the pinned Data Package

## `species.csv`

Percentage of year-round habitat for four species -- American robin, white-tailed deer, American bullfrog, and common gartersnake -- within US counties, derived from USGS Gap Analysis Project (GAP) Species Habitat Maps. Data is provided at a 30-meter resolution and covers the contiguous United States. Habitat percentages are calculated by overlaying species habitat rasters (year-round habitat represented by value 3) with US county boundaries. The habitat maps are in Albers Conical Equal Area projection (EPSG:5070). County boundaries are derived from US Census Bureau cartographic boundary files (1:10,000,000 scale), from `US-10m.json` in this repository. This dataset only includes *year-round* habitat. The original raster data also contains values for summer and winter habitat, which are *not* included in this dataset. Data was processed using the `exactextract` library for zonal statistics.

- Data Package name: `species`
- Format: `csv`
- Git-blob hash: `sha1:636fe2d2445d6fff0fa3c1d117457e83f68a6916`
- Licenses: [U.S. Government Dataset](https://www.usa.gov/government-works)
- Sources: [USGS Gap Analysis Project (GAP) Species Habitat Maps](https://www.usgs.gov/programs/gap-analysis-project), [US Census Bureau Cartographic Boundary Files (1:10,000,000)](https://www.census.gov/geographies/mapping-files/time-series/geo/cartographic-boundary.html)

## `stocks.csv`

Monthly stock prices for five companies from 2000 to 2010.

- Data Package name: `stocks`
- Format: `csv`
- Git-blob hash: `sha1:58e2ce1bed01eeebe29f5b4be32344aaec5532c0`
- Licenses: not specified in the pinned Data Package
- Sources: not specified in the pinned Data Package

## `unemployment-across-industries.json`

Industry-level unemployment from the Current Population Survey (CPS), published monthly by the U.S. Bureau of Labor Statistics. Includes unemployed persons and unemployment rate across 11 private industries, as well as agricultural, government, and self-employed workers. Covers January 2000 through February 2010. Industry classification follows format of CPS Table A-31. Transformed using `scripts/make-unemployment-across-industries.py` The BLS Web site states: > "Users of the public API should cite the date that data were accessed or retrieved using > the API. Users must clearly state that "BLS.gov cannot vouch for the data or analyses > derived from these data after the data have been retrieved from BLS.gov." The BLS.gov logo > may not be used by persons who are not BLS employees or on products (including web pages) > that are not BLS-sponsored." See full BLS [terms of service](https://www.bls.gov/developers/termsOfService.htm).

- Data Package name: `unemployment_across_industries`
- Format: `json`
- Git-blob hash: `sha1:4d769356c95c40a9807a7d048ab81aa56ae77df0`
- Licenses: [U.S. Government Dataset](https://www.usa.gov/government-works)
- Sources: [U.S. Census Bureau Current Population Survey](https://www.census.gov/programs-surveys/cps.html), [BLS LAUS Data Tools](https://www.bls.gov/lau/data.htm), [Bureau of Labor Statistics Table A-31](https://www.bls.gov/web/empsit/cpseea31.htm)

## `unemployment.tsv`

County-level unemployment rates in the United States, with data generally consistent with levels reported in 2009. The dataset is structured as tab-separated values. The unemployment rate represents the number of unemployed persons as a percentage of the labor force. According to the Bureau of Labor Statistics (BLS) glossary: Unemployed persons (Current Population Survey) [are] persons aged 16 years and older who had no employment during the reference week, were available for work, except for temporary illness, and had made specific efforts to find employment sometime during the 4-week period ending with the reference week. Persons who were waiting to be recalled to a job from which they had been laid off need not have been looking for work to be classified as unemployed. Derived from the [Local Area Unemployment Statistics (LAUS)](https://www.bls.gov/lau/) program, a federal-state cooperative effort overseen by the Bureau of Labor Statistics (BLS). The LAUS program produces monthly and annual employment, unemployment, and labor force data for census regions and divisions, states, counties, metropolitan areas, and many cities and towns. For the most up-to-date LAUS data: 1. **Monthly and Annual Data Downloads**: - Visit the [LAUS Data Tools](https://www.bls.gov/lau/data.htm) page for [monthly](https://www.bls.gov/lau/tables.htm#mcounty) and [annual](https://www.bls.gov/lau/tables.htm#cntyaa) county data. 2. **BLS Public Data API**: - The BLS provides an API for developers to access various datasets, including LAUS data. - To use the API for LAUS data, refer to the [LAUS Series ID Formats](https://www.bls.gov/help/hlpforma.htm#LA) to construct your query. - API documentation and examples are available on the BLS Developers page. When using BLS public data API and datasets, users should adhere to the [BLS Terms of Service](https://www.bls.gov/developers/termsOfService.htm).

- Data Package name: `unemployment`
- Format: `tsv`
- Git-blob hash: `sha1:d1aca19c4821fdc3b4270989661a1787d38588d0`
- Licenses: [U.S. Government Dataset](https://www.usa.gov/government-works)
- Sources: [BLS Developers API](https://www.bls.gov/developers/), [BLS Handbook of Methods](https://www.bls.gov/opub/hom/lau/home.htm)

## `us-10m.json`

US county boundaries represented at a 1:10,000,000 scale in [TopoJSON](https://github.com/topojson/topojson) format, which optimizes for smaller file sizes. Similar to offerings in the TopoJSON US Atlas collection, which in turn is a redistribution of the Census Bureau's cartographic boundary shapefiles.

- Data Package name: `us_10m`
- Format: `topojson`
- Git-blob hash: `sha1:ff7a7e679c46f2d1eb85cc92521b990f1a7a5c7a`
- Licenses: [TopoJSON US Atlas ISC License](https://github.com/topojson/us-atlas/blob/master/LICENSE)
- Sources: [TopoJSON US Atlas](https://github.com/topojson/us-atlas), [US Census Bureau Cartographic Boundary FIles](https://www.census.gov/geographies/mapping-files/time-series/geo/cartographic-boundary.html)

## `us-state-capitals.json`

Geographical coordinates and names of U.S. state capitals, transformed using `scripts/us-state-capitals.py`. Includes latitude, longitude, state name, and capital city name for all 50 U.S. states. Cities are represented as point locations of their capitol buildings using coordinates in the WGS84 geographic coordinate system. According to [USGS]((https://www.usgs.gov/faqs/what-are-terms-uselicensing-map-services-and-data-national-map)) > "Map services and data downloaded from The National Map are free and in the public domain. > There are no restrictions; however, we request that the following acknowledgment statement > of the originating agency be included in products and data derived from our map services > when citing, copying, or reprinting: Map services and data available from U.S. > Geological Survey, National Geospatial Program."

- Data Package name: `us_state_capitals`
- Format: `json`
- Git-blob hash: `sha1:32b4d3a13918b0aa85e62c09495eccf842fffb31`
- Licenses: [U.S. Public Domain](https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits), [U.S. Government Dataset](https://www.usa.gov/government-works)
- Sources: [U.S. Geological Survey National Geospatial Program - The National Map](https://www.usgs.gov/programs/national-geospatial-program/national-map)

## `weather.csv`

Daily weather observations from Seattle and New York. Transformed from NOAA data using the script `/scripts/weather.py`. The categorical "weather" field is synthesized from multiple fields in the original dataset. Intended for instructional purposes.

- Data Package name: `weather`
- Format: `csv`
- Git-blob hash: `sha1:0e7e853f4c5b67615da261d5d343824a43510f50`
- Licenses: [U.S. Government Dataset](https://www.usa.gov/government-works)
- Sources: [NOAA Climate Data Online](http://www.ncdc.noaa.gov/cdo-web/datatools/findstation)

## `weekly-weather.json`

Instructional dataset showing actual and predicted temperature data. > [!IMPORTANT] > Named `weather.json` in previous versions (`v1.4.0` - `v2.11.0`).

- Data Package name: `weekly_weather`
- Format: `json`
- Git-blob hash: `sha1:bd42a3e2403e7ccd6baaa89f93e7f0c164e0c185`
- Licenses: not specified in the pinned Data Package
- Sources: not specified in the pinned Data Package

## `wheat.json`

As noted by in this protovis [example](https://mbostock.github.io/protovis/ex/wheat.html), "In an 1822 letter to Parliament, [William Playfair](https://en.wikipedia.org/wiki/William_Playfair), a Scottish engineer who is often credited as the founder of statistical graphics, published an elegant chart on the price of wheat. It plots 250 years of prices alongside weekly wages and the reigning monarch. He intended to demonstrate that: > 'never at any former period was wheat so cheap, in proportion to mechanical labour, as it is at the present time.'"

- Data Package name: `wheat`
- Format: `json`
- Git-blob hash: `sha1:cde46b43fc82f4c3c2a37ddcfe99fd5f4d8d8791`
- Licenses: [Public Domain](https://commons.wikimedia.org/wiki/Public_domain)
- Sources: [1822 Playfair Chart](https://commons.wikimedia.org/wiki/File:Chart_Showing_at_One_View_the_Price_of_the_Quarter_of_Wheat,_and_Wages_of_Labour_by_the_Week,_from_1565_to_1821.png)

## `windvectors.csv`

Simulated wind patterns over northwestern Europe.

- Data Package name: `windvectors`
- Format: `csv`
- Git-blob hash: `sha1:ed686b0ba613abd59d09fcd946b5030a918b8154`
- Licenses: not specified in the pinned Data Package
- Sources: not specified in the pinned Data Package

## `world-110m.json`

A 1:110,000,000-scale world map in [TopoJSON](https://github.com/topojson/topojson) format, optimized for web-based visualization. The simplified geographic boundaries focus on two key elements: land masses and country borders with their corresponding codes. The high level of generalization removes small geographic details while maintaining recognizable global features, making it ideal for overview maps and basic world visualizations. This format provides efficient compression compared to GeoJSON, reducing file size for web use. Part of the widely-used TopoJSON World Atlas collection, this has become a standard resource for creating web-based world maps where precise boundary detail isn't required.

- Data Package name: `world_110m`
- Format: `topojson`
- Git-blob hash: `sha1:a1ce852de6f2713c94c0c284039506ca2d4f3dee`
- Licenses: [TopoJSON World Atlas ISC License](https://github.com/topojson/world-atlas/blob/master/LICENSE), [Natural Earth Data Public Domain](https://www.naturalearthdata.com/about/terms-of-use/)
- Sources: [TopoJSON World Atlas (Likely original source, processed from Natural Earth data)](https://github.com/topojson/world-atlas), [Natural Earth Data - Admin 0 Countries (1:110m)](https://www.naturalearthdata.com/downloads/110m-cultural-vectors/110m-admin-0-countries/)

## `zipcodes.csv`

Postal codes mapped to their geographical coordinates (latitude/longitude in WGS84) and administrative hierarchies, for the United States and Puerto Rico. The GeoNames geographical database provides worldwide postal code data with associated geographical and administrative information. Historical snapshot first contributed to vega-datasets in 2017 and no longer current. Administrative boundaries have been redrawn, counties reorganized and renamed, and postal codes modified. Latitude/longitude coordinates have been updated by Geonames since this data was collected. For current postal code data, refer to the main GeoNames database.

- Data Package name: `zipcodes`
- Format: `csv`
- Git-blob hash: `sha1:d3df33e12be0d0544c95f1bd47005add4b7010be`
- Licenses: [Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/)
- Sources: [GeoNames Postal Codes](https://download.geonames.org/export/zip/)
