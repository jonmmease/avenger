import * as wasm from "avenger-wasm";

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

export function registerVegaRenderer(renderModule) {
    // Call with renderModule function from 'vega-scenegraph'
    renderModule('avenger', {
        handler: AvengerHandler,
        renderer: AvengerRenderer
    });
}
