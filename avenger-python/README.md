# Avenger: A visualization engine and renderer
Avenger is an early stage prototype of a new foundational rendering library for information visualization (InfoVis) systems. Avenger defines a 2D scenegraph representation tailored to the needs of InfoVis systems. To start with, the initial application of Avenger is to serve as an alternative, GPU accelerated, rendering backend for Vega visualizations.

# Try it out in Python with Vega-Altair
The `avenger` Python package provides a [custom Altair renderer](https://altair-viz.github.io/user_guide/custom_renderers.html) named `avenger-png`. This renderer relies on vl-convert to extract the vega scenegraph corresponding to a chart and then uses Avenger to render the chart to a static PNG image.

First, install altair, vega-datasets, avenger, and vl-convert-python
```
pip install -U altair vega_datasets avenger "vl-convert-python>=1.2.3"
```

Then import Altair and activate the `avenger-png` renderer

```python
import altair as alt
alt.renderers.enable('avenger-png', scale=1)
```

Then create and display an Altair chart as usual:

```python
import altair as alt
from vega_datasets import data

source = data.cars()

chart = alt.Chart(source).mark_circle(size=60).encode(
    x='Horsepower',
    y='Miles_per_Gallon',
    color='Origin',
)
chart
```
![cars_scatter](https://github.com/jonmmease/avenger/assets/15064365/d661e142-c7c5-4816-a375-49a73985bb6d)

Or, convert the chart to a PNG rendered by Avenger:

```python
import avenger
png = avenger.altair_utils.chart_to_png(chart, scale=1)
with open("scatter.png", "wb") as f:
    f.write(png)
```
## Comparison to vl-convert
There aren't currently many advantages to using Avenger to render Altar charts to PNG as compared with vl-convert, which performs rendering using [resvg](https://github.com/RazrFalcon/resvg). Performance is generally comparable, though Avenger can be a bit faster for charts with a large number of symbol instances.

One advantage is that Avenger's text rendering support is based on [COSMIC Text](https://github.com/pop-os/cosmic-text), which supports emoji (unlike resvg's text handling). For example, here is the result of rendering the emoji example from https://altair-viz.github.io/gallery/isotype_emoji.html using Avenger:

![isotype_emoji](https://github.com/jonmmease/avenger/assets/15064365/91a1db89-9bdd-46f3-b540-c7d7bcaac3c2)
