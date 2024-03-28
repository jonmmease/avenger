import vegaEmbed from 'vega-embed';
import { registerVegaRenderer, viewToPng } from 'avenger-wasm';
import { renderModule } from 'vega-scenegraph';

const carsData = require('./data/cars_40k.json');

var spec = {
    "$schema": "https://vega.github.io/schema/vega/v5.json",
    "background": "white",
    "padding": 5,
    "width": 400,
    "height": 400,
    "style": "cell",
    "data": [
        {"name": "grid_store"},
        {
            "name": "source_0",
            "values": carsData,
            "format": {"type": "json"},
            "transform": [
                {
                    "type": "filter",
                    "expr": "isValid(datum[\"Horsepower\"]) && isFinite(+datum[\"Horsepower\"]) && isValid(datum[\"Miles_per_Gallon\"]) && isFinite(+datum[\"Miles_per_Gallon\"]) && isValid(datum[\"Cylinders\"]) && isFinite(+datum[\"Cylinders\"])"
                }
            ]
        }
    ],
    "signals": [
        {
            "name": "unit",
            "value": {},
            "on": [
                {"events": "pointermove", "update": "isTuple(group()) ? group() : unit"}
            ]
        },
        {"name": "grid", "update": "vlSelectionResolve(\"grid_store\", \"union\")"},
        {
            "name": "grid_Horsepower",
            "on": [
                {"events": [{"source": "view", "type": "dblclick"}], "update": "null"},
                {
                    "events": {"signal": "grid_translate_delta"},
                    "update": "panLinear(grid_translate_anchor.extent_x, -grid_translate_delta.x / width)"
                },
                {
                    "events": {"signal": "grid_zoom_delta"},
                    "update": "zoomLinear(domain(\"x\"), grid_zoom_anchor.x, grid_zoom_delta)"
                }
            ]
        },
        {
            "name": "grid_Miles_per_Gallon",
            "on": [
                {"events": [{"source": "view", "type": "dblclick"}], "update": "null"},
                {
                    "events": {"signal": "grid_translate_delta"},
                    "update": "panLinear(grid_translate_anchor.extent_y, grid_translate_delta.y / height)"
                },
                {
                    "events": {"signal": "grid_zoom_delta"},
                    "update": "zoomLinear(domain(\"y\"), grid_zoom_anchor.y, grid_zoom_delta)"
                }
            ]
        },
        {
            "name": "grid_tuple",
            "on": [
                {
                    "events": [{"signal": "grid_Horsepower || grid_Miles_per_Gallon"}],
                    "update": "grid_Horsepower && grid_Miles_per_Gallon ? {unit: \"\", fields: grid_tuple_fields, values: [grid_Horsepower,grid_Miles_per_Gallon]} : null"
                }
            ]
        },
        {
            "name": "grid_tuple_fields",
            "value": [
                {"field": "Horsepower", "channel": "x", "type": "R"},
                {"field": "Miles_per_Gallon", "channel": "y", "type": "R"}
            ]
        },
        {
            "name": "grid_translate_anchor",
            "value": {},
            "on": [
                {
                    "events": [{"source": "scope", "type": "pointerdown"}],
                    "update": "{x: x(unit), y: y(unit), extent_x: domain(\"x\"), extent_y: domain(\"y\")}"
                }
            ]
        },
        {
            "name": "grid_translate_delta",
            "value": {},
            "on": [
                {
                    "events": [
                        {
                            "source": "window",
                            "type": "pointermove",
                            "consume": true,
                            "between": [
                                {"source": "scope", "type": "pointerdown"},
                                {"source": "window", "type": "pointerup"}
                            ]
                        }
                    ],
                    "update": "{x: grid_translate_anchor.x - x(unit), y: grid_translate_anchor.y - y(unit)}"
                }
            ]
        },
        {
            "name": "grid_zoom_anchor",
            "on": [
                {
                    "events": [{"source": "scope", "type": "wheel", "consume": true}],
                    "update": "{x: invert(\"x\", x(unit)), y: invert(\"y\", y(unit))}"
                }
            ]
        },
        {
            "name": "grid_zoom_delta",
            "on": [
                {
                    "events": [{"source": "scope", "type": "wheel", "consume": true}],
                    "force": true,
                    "update": "pow(1.001, event.deltaY * pow(16, event.deltaMode))"
                }
            ]
        },
        {
            "name": "grid_modify",
            "on": [
                {
                    "events": {"signal": "grid_tuple"},
                    "update": "modify(\"grid_store\", grid_tuple, true)"
                }
            ]
        }
    ],
    "marks": [
        {
            "name": "marks",
            "type": "symbol",
            "clip": true,
            "style": ["circle"],
            "interactive": true,
            "from": {"data": "source_0"},
            "encode": {
                "update": {
                    "opacity": {"value": 0.7},
                    "fill": {"value": "#4c78a8"},
                    // "ariaRoleDescription": {"value": "circle"},
                    // "description": {
                    //     "signal": "\"Horsepower: \" + (format(datum[\"Horsepower\"], \"\")) + \"; Miles_per_Gallon: \" + (format(datum[\"Miles_per_Gallon\"], \"\")) + \"; Cylinders: \" + (format(datum[\"Cylinders\"], \"\"))"
                    // },
                    "x": {"scale": "x", "field": "Horsepower"},
                    "y": {"scale": "y", "field": "Miles_per_Gallon"},
                    "size": {"scale": "size", "field": "Cylinders"},
                    "shape": {"value": "circle"}
                }
            }
        }
    ],
    "scales": [
        {
            "name": "x",
            "type": "linear",
            "domain": [75, 150],
            "domainRaw": {"signal": "grid[\"Horsepower\"]"},
            "range": [0, {"signal": "width"}],
            "zero": false
        },
        {
            "name": "y",
            "type": "linear",
            "domain": [20, 40],
            "domainRaw": {"signal": "grid[\"Miles_per_Gallon\"]"},
            "range": [{"signal": "height"}, 0],
            "zero": false
        },
        {
            "name": "size",
            "type": "linear",
            "domain": {"data": "source_0", "field": "Cylinders"},
            "range": [0, 361],
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
            "labels": false,
            "labelFlush": true,
            "labelOverlap": true,
            "tickCount": {"signal": "ceil(width/40)"},
            "zindex": 0
        },
        {
            "scale": "y",
            "orient": "left",
            "grid": false,
            "labels": false,
            "labelOverlap": true,
            "tickCount": {"signal": "ceil(height/40)"},
            "zindex": 0
        }
    ]
};

// var spec = {
//     "$schema": "https://vega.github.io/schema/vega/v5.json",
//     "description": "A basic stacked bar chart example.",
//     "width": 500,
//     "height": 200,
//     "padding": 5,
//     "background": "white",
//     "data": [
//         {
//             "name": "table",
//             "values": [
//                 {"x": 0, "y": 28, "c": 0}, {"x": 0, "y": 55, "c": 1},
//                 {"x": 1, "y": 43, "c": 0}, {"x": 1, "y": 91, "c": 1},
//                 {"x": 2, "y": 81, "c": 0}, {"x": 2, "y": 53, "c": 1},
//                 {"x": 3, "y": 19, "c": 0}, {"x": 3, "y": 87, "c": 1},
//                 {"x": 4, "y": 52, "c": 0}, {"x": 4, "y": 48, "c": 1},
//                 {"x": 5, "y": 24, "c": 0}, {"x": 5, "y": 49, "c": 1},
//                 {"x": 6, "y": 87, "c": 0}, {"x": 6, "y": 66, "c": 1},
//                 {"x": 7, "y": 17, "c": 0}, {"x": 7, "y": 27, "c": 1},
//                 {"x": 8, "y": 68, "c": 0}, {"x": 8, "y": 16, "c": 1},
//                 {"x": 9, "y": 49, "c": 0}, {"x": 9, "y": 15, "c": 1}
//             ],
//             "transform": [
//                 {
//                     "type": "stack",
//                     "groupby": ["x"],
//                     "sort": {"field": "c"},
//                     "field": "y"
//                 }
//             ]
//         }
//     ],
//
//     "scales": [
//         {
//             "name": "x",
//             "type": "band",
//             "range": "width",
//             "domain": {"data": "table", "field": "x"}
//         },
//         {
//             "name": "y",
//             "type": "linear",
//             "range": "height",
//             "nice": true, "zero": true,
//             "domain": {"data": "table", "field": "y1"}
//         },
//         {
//             "name": "color",
//             "type": "ordinal",
//             "range": "category",
//             "domain": {"data": "table", "field": "c"}
//         },
//         {
//             "name": "radius",
//             "type": "ordinal",
//             "range": [6, 12],
//             "domain": {"data": "table", "field": "c"}
//         }
//     ],
//
//     "marks": [
//         {
//             "type": "rect",
//             "from": {"data": "table"},
//             "encode": {
//                 "enter": {
//                     "x": {"scale": "x", "field": "x"},
//                     "width": {"scale": "x", "band": 1, "offset": -10},
//                     "y": {"scale": "y", "field": "y0"},
//                     "y2": {"scale": "y", "field": "y1"},
//                     "fill": {"scale": "color", "field": "c"},
//                     "cornerRadius": {"scale": "radius", "field": "c"}
//                 },
//                 "update": {
//                     "fillOpacity": {"value": 1}
//                 }
//             }
//         }
//     ]
// };

registerVegaRenderer(renderModule);

vegaEmbed('#plot-container', spec, {
    // renderer: "canvas",
    renderer: "avenger",
}).then((result) => {
    // Access the Vega view instance (https://vega.github.io/vega/docs/api/view/) as result.view
    return viewToPng(result.view);
}).then((png) => {
    console.log("The PNG:", png);
}).catch(console.error);
