import * as wasm from "avenger-wasm";

import { Bounds, CanvasRenderer, Renderer, Handler, CanvasHandler, renderModule, domClear, domChild } from 'vega-scenegraph';
import { inherits } from 'vega-util';
import vegaEmbed from 'vega-embed';

const carsData = require('./data/cars_40k.json');

function devicePixelRatio() {
    return typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1;
}

function resize(canvas, width, height, origin, scaleFactor, opt) {
    const inDOM = typeof HTMLElement !== 'undefined'
            && canvas instanceof HTMLElement
            && canvas.parentNode != null,
        context = canvas.getContext('2d'),
        ratio = inDOM ? devicePixelRatio() : scaleFactor;

    canvas.width = width * ratio;
    canvas.height = height * ratio;

    for (const key in opt) {
        context[key] = opt[key];
    }

    if (inDOM && ratio !== 1) {
        canvas.style.width = width + 'px';
        canvas.style.height = height + 'px';
    }

    context.pixelRatio = ratio;
    context.setTransform(
        ratio, 0, 0, ratio,
        ratio * origin[0],
        ratio * origin[1]
    );

    return canvas;
}


function ProfileCanvasRenderer(loader) {
    CanvasRenderer.call(this, loader);
}

inherits(ProfileCanvasRenderer, CanvasRenderer, {
    _render(scene) {
        var start = performance.now();
        CanvasRenderer.prototype._render.call(this, scene);
        console.log("_render time: " + (performance.now() - start));
        return this;
    }
})

renderModule('profile-canvas', {
    handler: CanvasHandler,
    renderer: ProfileCanvasRenderer
});

export default function AvengerRenderer(loader) {
    Renderer.call(this, loader);
}

let base = Renderer.prototype;

inherits(AvengerRenderer, Renderer, {
    initialize(el, width, height, origin) {
        this._width = width;
        this._height = height;
        this._origin = origin;

        this._root_el = domChild(el, 0, 'div');
        this._root_el.style.position = 'relative';

        // Create overlayed div elements
        const bottomEl = domChild(this._root_el, 0, 'div');
        const topEl = domChild(this._root_el, 1, 'div');
        bottomEl.style.height = '100%';
        topEl.style.position = 'absolute';
        topEl.style.top = '0';
        topEl.style.left = '0';
        topEl.style.height = '100%';
        topEl.style.width = '100%';

        // Add Avenger canvas to bottom element
        this._avengerHtmlCanvas = document.createElement('canvas');
        domClear(bottomEl, 0).appendChild(this._avengerHtmlCanvas);
        this._avengerHtmlCanvas.setAttribute('class', 'marks');

        // Add event canvas to top element
        this._handlerCanvas = document.createElement('canvas');
        domClear(topEl, 0).appendChild(this._handlerCanvas);
        this._handlerCanvas.setAttribute('class', 'marks');

        // Create Avenger canvas
        this._avengerCanvasPromise = new wasm.AvengerCanvas(this._avengerHtmlCanvas, width, height, origin[0], origin[1]);
        this._avengerCanvasPromise.then((avegnerCanvas) => {
            this._avengerCanvas = avegnerCanvas;
        });

        this._lastRenderFinishTime = performance.now();

        // this method will invoke resize to size the canvas appropriately
        return base.initialize.call(this, el, width, height, origin);
    },

    canvas() {
        return this._handlerCanvas
    },

    resize(width, height, origin) {
        this._width = width;
        this._height = height;
        this._origin = origin;

        base.resize.call(this, width, height, origin);
        resize(this._handlerCanvas, width, height, origin);

        // stuff
        return this;
    },

    _render(scene) {
        console.log("scene graph construction time: " + (performance.now() - this._lastRenderFinishTime));
        if (this._avengerCanvas) {
            var start = performance.now();
            const sceneGraph = importScenegraph(scene, this._width, this._height, this._origin);
            this._avengerCanvas.set_scene(sceneGraph);
            console.log("_render time: " + (performance.now() - start));
        }
        this._lastRenderFinishTime = performance.now();
        return this;
    }
})

export function AvengerHandler(loader, tooltip) {
    CanvasHandler.call(this, loader, tooltip);
}

inherits(AvengerHandler, CanvasHandler, {
    initialize(el, origin, obj) {
        const canvas = domChild(domChild(el, 0, 'div'), 1, 'div');
        return CanvasHandler.prototype.initialize.call(this, canvas, origin, obj);
    }
});

renderModule('avenger', {
    handler: AvengerHandler,
    renderer: AvengerRenderer
});

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

function importScenegraph(vegaSceneGroups, width, height, origin) {
    const sceneGraph = new wasm.SceneGraph(width, height, origin[0], origin[1]);
    for (const vegaGroup of vegaSceneGroups.items) {
        sceneGraph.add_group(importGroup(vegaGroup));
    }
    return sceneGraph;
}

function importGroup(vegaGroup) {
    const groupMark = new wasm.GroupMark(vegaGroup.x, vegaGroup.y, vegaGroup.name);

    for (const vegaMark of vegaGroup.items) {
        switch (vegaMark.marktype) {
            case "symbol":
                groupMark.add_symbol_mark(importSymbol(vegaMark));
                break;
            // case "rule":
            //     1
            default:
                console.log("Unsupported mark type: " + vegaMark.marktype)
        }
    }

    return groupMark;
}

function importSymbol(vegaSymbolMark) {
    const len = vegaSymbolMark.items.length;
    const symbolMark = new wasm.SymbolMark(len, vegaSymbolMark.clip, vegaSymbolMark.name);

    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);
    const size = new Float32Array(len).fill(20);
    const angle = new Float32Array(len).fill(0);
    const items = vegaSymbolMark.items;
    items.forEach((item, i) => {
        x[i] = item.x;
        y[i] = item.y;
        size[i] = item.size;
        if (item.angle) {
            angle[i] = item.angle;
        }
    })

    symbolMark.set_xy(x, y);
    symbolMark.set_size(size);
    symbolMark.set_angle(angle);

    return symbolMark;
}

const symbolMark = new wasm.SymbolMark(2, false, "hello");

const xs = new Float32Array([1, 2, 3]);
const ys = new Float32Array([10, 11, 12]);
symbolMark.set_xy(xs, ys);

console.log(xs);

const group = new wasm.GroupMark(10, 10, null);
group.add_symbol_mark(symbolMark);

const sceneGraph = new wasm.SceneGraph(300, 300, 10, 10);
sceneGraph.add_group(group);


vegaEmbed('#plot-container', spec, {
    // renderer: "canvas",
    renderer: "avenger",
}).then(function(result) {
    // Access the Vega view instance (https://vega.github.io/vega/docs/api/view/) as result.view
}).catch(console.error);
