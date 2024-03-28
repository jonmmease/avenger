import * as wasm from "../pkg/avenger_wasm";

import { Renderer, CanvasHandler, domClear, domChild } from 'vega-scenegraph';
import { inherits } from 'vega-util';


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

export default function AvengerRenderer(loader) {
    Renderer.call(this, loader);
}

let base = Renderer.prototype;

inherits(AvengerRenderer, Renderer, {
    initialize(el, width, height, origin) {
        this._width = width;
        this._height = height;
        this._origin = origin;
        this._last_scene = null;

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
        console.log("create: ", width, height, origin);
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

        this._avengerCanvasPromise = new wasm.AvengerCanvas(this._avengerHtmlCanvas, width, height, origin[0], origin[1]);
        this._avengerCanvas = null;
        this._avengerCanvasPromise.then((avegnerCanvas) => {
            this._avengerCanvas = avegnerCanvas;
            if (this._last_scene != null) {
                this._render(this._last_scene);
            }
        });

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
            this._last_scene = null;
        } else {
            // Canvas is being constructed after resize, save for render after construction complete
            this._last_scene = scene;
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

function importScenegraph(vegaSceneGroups, width, height, origin) {
    const sceneGraph = new wasm.SceneGraph(width, height, origin[0], origin[1]);
    for (const vegaGroup of vegaSceneGroups.items) {
        sceneGraph.add_group(importGroup(vegaGroup));
    }
    return sceneGraph;
}

function importGroup(vegaGroup) {
    const groupMark = new wasm.GroupMark(
        vegaGroup.x, vegaGroup.y, vegaGroup.name, vegaGroup.width, vegaGroup.height
    );

    for (const vegaMark of vegaGroup.items) {
        switch (vegaMark.marktype) {
            case "symbol":
                groupMark.add_symbol_mark(importSymbol(vegaMark));
                break;
            case "rule":
                groupMark.add_rule_mark(importRule(vegaMark));
                break;
            case "group":
                for (const groupItem of vegaMark.items) {
                    groupMark.add_group_mark(importGroup(groupItem));
                }
                break;
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
    const opacity = new Float32Array(len).fill(1);
    const fill = new Array(len).fill("");

    const items = vegaSymbolMark.items;
    items.forEach((item, i) => {
        x[i] = item.x;
        y[i] = item.y;
        size[i] = item.size;
        if (item.angle != null) {
            angle[i] = item.angle;
        }

        if (item.opacity != null) {
            opacity[i] = item.opacity;
        }

        if (item.fill != null) {
            fill[i] = item.fill;
        }
    })

    symbolMark.set_xy(x, y);
    symbolMark.set_size(size);
    symbolMark.set_angle(angle);

    // encode and set fill
    const encodedStroke = encodeStringArray(fill);
    symbolMark.set_fill(encodedStroke.joinedUniqueString, encodedStroke.indices, opacity);

    return symbolMark;
}

function importRule(vegaRuleMark) {
    const len = vegaRuleMark.items.length;
    const ruleMark = new wasm.RuleMark(len, vegaRuleMark.clip, vegaRuleMark.name);

    const x0 = new Float32Array(len).fill(0);
    const y0 = new Float32Array(len).fill(0);
    const x1 = new Float32Array(len).fill(0);
    const y1 = new Float32Array(len).fill(0);
    const width = new Float32Array(len).fill(1);
    const opacity = new Float32Array(len).fill(1);
    const stroke = new Array(len).fill("");

    const items = vegaRuleMark.items;
    items.forEach((item, i) => {
        if (item.x != null) {
            x0[i] = item.x;
        }
        if (item.y != null) {
            y0[i] = item.y;
        }
        if (item.x2 != null) {
            x1[i] = item.x2;
        } else {
            x1[i] = x0[i];
        }
        if (item.y2 != null) {
            y1[i] = item.y2;
        } else {
            y1[i] = y0[i];
        }
        if (item.width != null) {
            width[i] = item.width;
        }
        if (item.opacity != null) {
            opacity[i] = item.opacity;
        }
        if (item.stroke != null) {
            stroke[i] = item.stroke;
        }
    })

    ruleMark.set_xy(x0, y0, x1, y1);
    ruleMark.set_stroke_width(width);

    // encode and set stroke
    const encodedStroke = encodeStringArray(stroke);
    ruleMark.set_stroke(encodedStroke.joinedUniqueString, encodedStroke.indices, opacity);

    return ruleMark;
}

function encodeStringArray(originalArray) {
    const uniqueStringsMap = new Map();
    let index = 0;

    // Populate the map with unique strings and their indices
    for (const str of originalArray) {
        if (!uniqueStringsMap.has(str)) {
            uniqueStringsMap.set(str, index++);
        }
    }

    // Generate the array of unique strings.
    // Note, Maps preserve the insertion order of their elements
    const uniqueStringsArray = Array.from(uniqueStringsMap.keys());

    // Build index array
    let indices = new Uint32Array(originalArray.length);
    originalArray.forEach((str, i) => {
        indices[i] = uniqueStringsMap.get(str);
    });

    return {
        joinedUniqueString: uniqueStringsArray.join(":"),
        indices,
    };
}

// Example usage
const originalArray = ["apple", "banana", "apple", "orange", "banana", "apple"];
const result = encodeStringArray(originalArray);

console.log(result.uniqueStringsArray); // ["apple", "banana", "orange"]
console.log(result.indices); // TypedArray of indices


export function registerVegaRenderer(renderModule) {
    // Call with renderModule function from 'vega-scenegraph'
    renderModule('avenger', {
        handler: AvengerHandler,
        renderer: AvengerRenderer
    });
}

