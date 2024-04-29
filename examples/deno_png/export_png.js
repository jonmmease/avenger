import * as vega from "https://cdn.skypack.dev/pin/vega@v5.28.0-apQC2txkSWyYRCV9ipfx/mode=imports/optimized/vega.js"
import { viewToPng } from "../../avenger-vega-renderer/dist-deno/js/index.js"

var spec = {
    "$schema": "https://vega.github.io/schema/vega/v5.json",
    "description": "A scatterplot showing horsepower and miles per gallons for various cars.",
    "background": "white",
    "padding": 5,
    "width": 200,
    "height": 200,
    "style": "cell",
    "data": [
        {
            "name": "source_0",
            "url": "https://raw.githubusercontent.com/vega/vega-datasets/main/data/cars.json",
            "format": {"type": "json"},
            "transform": [
                {
                    "type": "filter",
                    "expr": "isValid(datum[\"Horsepower\"]) && isFinite(+datum[\"Horsepower\"]) && isValid(datum[\"Miles_per_Gallon\"]) && isFinite(+datum[\"Miles_per_Gallon\"])"
                }
            ]
        }
    ],
    "marks": [
        {
            "name": "marks",
            "type": "symbol",
            "style": ["circle"],
            "from": {"data": "source_0"},
            "encode": {
                "update": {
                    "opacity": {"value": 0.7},
                    "fill": {"value": "#4c78a8"},
                    "ariaRoleDescription": {"value": "circle"},
                    "x": {"scale": "x", "field": "Horsepower"},
                    "y": {"scale": "y", "field": "Miles_per_Gallon"},
                    "shape": {"value": "circle"}
                }
            }
        }
    ],
    "scales": [
        {
            "name": "x",
            "type": "linear",
            "domain": {"data": "source_0", "field": "Horsepower"},
            "range": [0, {"signal": "width"}],
            "nice": true,
            "zero": true
        },
        {
            "name": "y",
            "type": "linear",
            "domain": {"data": "source_0", "field": "Miles_per_Gallon"},
            "range": [{"signal": "height"}, 0],
            "nice": true,
            "zero": true
        }
    ],
    "axes": [
        {
            "scale": "x",
            "orient": "bottom",
            "gridScale": "y",
            "grid": true,
            "tickCount": {"signal": "ceil(width/40)"},
            "domain": false,
            "labels": false,
            "aria": false,
            "maxExtent": 0,
            "minExtent": 0,
            "ticks": false,
            "zindex": 0
        },
        {
            "scale": "y",
            "orient": "left",
            "gridScale": "x",
            "grid": true,
            "tickCount": {"signal": "ceil(height/40)"},
            "domain": false,
            "labels": false,
            "aria": false,
            "maxExtent": 0,
            "minExtent": 0,
            "ticks": false,
            "zindex": 0
        },
        {
            "scale": "x",
            "orient": "bottom",
            "grid": false,
            "labelFlush": true,
            "labelOverlap": true,
            "labels": false,
            "tickCount": {"signal": "ceil(width/40)"},
            "zindex": 0
        },
        {
            "scale": "y",
            "orient": "left",
            "grid": false,
            "labelOverlap": true,
            "labels": false,
            "tickCount": {"signal": "ceil(height/40)"},
            "zindex": 0
        }
    ]
};

const runtime = vega.parse(spec);
const view = new vega.View(runtime, {renderer: 'none'});
const png = await viewToPng(view);
Deno.writeFile("chart.png", png);
