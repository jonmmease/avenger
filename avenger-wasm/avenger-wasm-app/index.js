import * as wasm from "avenger-wasm";
import { Bounds, Renderer, Handler, renderModule, domClear } from 'vega-scenegraph';
import { inherits } from 'vega-util';
import vegaEmbed from 'vega-embed';

function cloneScenegraph(obj) {
    const keys = [
        'marktype', 'name', 'role', 'interactive', 'clip', 'items', 'zindex',
        'x', 'y', 'width', 'height', 'align', 'baseline',             // layout
        'fill', 'fillOpacity', 'opacity', 'blend',                    // fill
        'x1', 'y1', 'r1', 'r2', 'gradient',                           // gradient
        'stops', 'offset', 'color',
        'stroke', 'strokeOpacity', 'strokeWidth', 'strokeCap',        // stroke
        'strokeJoin',
        'strokeDash', 'strokeDashOffset',                             // stroke dash
        'strokeForeground', 'strokeOffset',                           // group
        'startAngle', 'endAngle', 'innerRadius', 'outerRadius',       // arc
        'cornerRadius', 'padAngle',                                   // arc, rect
        'cornerRadiusTopLeft', 'cornerRadiusTopRight',                // rect, group
        'cornerRadiusBottomLeft', 'cornerRadiusBottomRight',
        'interpolate', 'tension', 'orient', 'defined',                // area, line
        'url', 'aspect', 'smooth',                                    // image
        'path', 'scaleX', 'scaleY',                                   // path
        'x2', 'y2',                                                   // rule
        'size', 'shape',                                              // symbol
        'text', 'angle', 'theta', 'radius', 'dir', 'dx', 'dy',        // text
        'ellipsis', 'limit', 'lineBreak', 'lineHeight',
        'font', 'fontSize', 'fontWeight', 'fontStyle', 'fontVariant', // font
        'description', 'aria', 'ariaRole', 'ariaRoleDescription'      // aria
    ];

    // Check if the input is an object (including an array) or null
    if (typeof obj !== 'object' || obj === null) {
        return obj;
    }

    // Initialize the clone as an array or object based on the input type
    const clone = Array.isArray(obj) ? [] : {};

    // If the object is an array, iterate over its elements
    if (Array.isArray(obj)) {
        for (let i = 0; i < obj.length; i++) {
            // Apply the function recursively to each element
            clone.push(cloneScenegraph(obj[i]));
        }
    } else {
        // If the object is not an array, iterate over its keys
        for (const key in obj) {
            // Clone only the properties with specified keys
            if (key === "shape" && typeof obj[key] === "function") {
                // Convert path object to SVG path string.
                // Initialize context. This is needed for obj.shape(obj) to work.
                obj.shape.context();
                clone["shape"] = obj.shape(obj) ?? "";
            } else if (keys.includes(key)) {
                clone[key] = cloneScenegraph(obj[key]);
            }
        }
    }

    return clone;
}

export default function AvengerRenderer(loader) {
    Renderer.call(this, loader);
    this._options = {};
    this._redraw = false;
    this._dirty = new Bounds();
    this._tempb = new Bounds();
}

let base = Renderer.prototype;

inherits(AvengerRenderer, Renderer, {
    initialize(el, width, height, origin) {
        this._canvas = document.createElement('canvas'); // instantiate a small canvas

        if (el && this._canvas) {
            domClear(el, 0).appendChild(this._canvas);
            this._canvas.setAttribute('class', 'marks');
        }

        console.log("init", width, height, origin);
        // stuff
        this._avengerCanvasPromise = new wasm.AvengerCanvas(this._canvas, width, height, origin[0], origin[1]);

        // this method will invoke resize to size the canvas appropriately
        return base.initialize.call(this, el, width, height, origin);
    },

    canvas() {
        return this._canvas
    },

    resize(width, height, origin) {
        base.resize.call(this, width, height, origin);
        console.log("resize", width, height, origin);

        // stuff
        return this;
    },

    _render(scene) {
        const cleanedSceneGrpah = cloneScenegraph(scene);
        this._avengerCanvasPromise.then((avangerCanvas) => {
            console.log("set_scene");
            avangerCanvas.set_scene(cleanedSceneGrpah);
        })

        console.log("render");
        return this;
    }
})

export function AvengerHandler(loader) {
    Handler.call(this, loader);
}

inherits(AvengerHandler, Handler, {
    initialize(el, origin, obj) {
        console.log("AvengerHandler.initialize", el, origin, obj);
        this._renderer = obj._renderer;
        return Handler.prototype.initialize.call(this, el, origin, obj);
    },
    on(type, handler) {
        console.log("on", type, this._renderer);
    },
    off(type, handler) {
        console.log("off", type);
    }
});

renderModule('avenger', {
    handler: AvengerHandler,
    renderer: AvengerRenderer
});

var spec = {
    "$schema": "https://vega.github.io/schema/vega/v5.json",
    "description": "A basic stacked bar chart example.",
    "width": 500,
    "height": 200,
    "padding": 5,
    "background": "white",
    "data": [
        {
            "name": "table",
            "values": [
                {"x": 0, "y": 28, "c": 0}, {"x": 0, "y": 55, "c": 1},
                {"x": 1, "y": 43, "c": 0}, {"x": 1, "y": 91, "c": 1},
                {"x": 2, "y": 81, "c": 0}, {"x": 2, "y": 53, "c": 1},
                {"x": 3, "y": 19, "c": 0}, {"x": 3, "y": 87, "c": 1},
                {"x": 4, "y": 52, "c": 0}, {"x": 4, "y": 48, "c": 1},
                {"x": 5, "y": 24, "c": 0}, {"x": 5, "y": 49, "c": 1},
                {"x": 6, "y": 87, "c": 0}, {"x": 6, "y": 66, "c": 1},
                {"x": 7, "y": 17, "c": 0}, {"x": 7, "y": 27, "c": 1},
                {"x": 8, "y": 68, "c": 0}, {"x": 8, "y": 16, "c": 1},
                {"x": 9, "y": 49, "c": 0}, {"x": 9, "y": 15, "c": 1}
            ],
            "transform": [
                {
                    "type": "stack",
                    "groupby": ["x"],
                    "sort": {"field": "c"},
                    "field": "y"
                }
            ]
        }
    ],

    "scales": [
        {
            "name": "x",
            "type": "band",
            "range": "width",
            "domain": {"data": "table", "field": "x"}
        },
        {
            "name": "y",
            "type": "linear",
            "range": "height",
            "nice": true, "zero": true,
            "domain": {"data": "table", "field": "y1"}
        },
        {
            "name": "color",
            "type": "ordinal",
            "range": "category",
            "domain": {"data": "table", "field": "c"}
        },
        {
            "name": "radius",
            "type": "ordinal",
            "range": [6, 12],
            "domain": {"data": "table", "field": "c"}
        }
    ],

    "marks": [
        {
            "type": "rect",
            "from": {"data": "table"},
            "encode": {
                "enter": {
                    "x": {"scale": "x", "field": "x"},
                    "width": {"scale": "x", "band": 1, "offset": -10},
                    "y": {"scale": "y", "field": "y0"},
                    "y2": {"scale": "y", "field": "y1"},
                    "fill": {"scale": "color", "field": "c"},
                    "cornerRadius": {"scale": "radius", "field": "c"}
                },
                "update": {
                    "fillOpacity": {"value": 1}
                }
            }
        }
    ]
};

vegaEmbed('#plot-container', spec, {renderer: "avenger"}).then(function(result) {
    // Access the Vega view instance (https://vega.github.io/vega/docs/api/view/) as result.view
}).catch(console.error);
