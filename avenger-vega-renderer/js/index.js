import init, { AvengerCanvas, scene_graph_to_png } from "../lib/avenger_vega_renderer.js";
import { Renderer, CanvasHandler, domClear, domChild } from 'vega-scenegraph';
import { inherits } from 'vega-util';
import { importScenegraph } from "./marks/scenegraph.js"

const AVENGER_OPTIONS = {}

await init();

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
        this._avengerHtmlCanvas.style.width = width + "px";
        this._avengerHtmlCanvas.style.height = height + "px";

        // Add event canvas to top element
        this._handlerCanvas = document.createElement('canvas');
        domClear(topEl, 0).appendChild(this._handlerCanvas);
        this._handlerCanvas.setAttribute('class', 'marks');

        // Create Avenger canvas
        this._avengerCanvasPromise = new AvengerCanvas(this._avengerHtmlCanvas, width, height, origin[0], origin[1]);

        this._lastRenderFinishTime = performance.now();

        // this method will invoke resize to size the canvas appropriately
        return base.initialize.call(this, el, width, height, origin);
    },

    canvas() {
        return this._handlerCanvas
    },

    resize(width, height, origin) {
        this._avengerHtmlCanvas.style.width = width + "px";
        this._avengerHtmlCanvas.style.height = height + "px";
        this._avengerCanvasPromise = new AvengerCanvas(this._avengerHtmlCanvas, width, height, origin[0], origin[1]);

        base.resize.call(this, width, height, origin);
        resize(this._handlerCanvas, width, height, origin);

        return this;
    },

    _render(scene) {
        this.log("scene graph construction time: " + (performance.now() - this._lastRenderFinishTime))
        this._avengerCanvasPromise.then((avengerCanvas) => {
            var start = performance.now();
            importScenegraph(
                scene,
                avengerCanvas.width(),
                avengerCanvas.height(),
                [avengerCanvas.origin_x(), avengerCanvas.origin_y()],
                this._loader,
            ).then((sceneGraph) => {
                avengerCanvas.set_scene(sceneGraph);
                this.log("_render time: " + (performance.now() - start));
            });
        });
        this._lastRenderFinishTime = performance.now();
        return this;
    },

    log(msg) {
        if (AVENGER_OPTIONS["verbose"]) {
            console.log(msg);
        }
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

export function registerVegaRenderer(renderModule, verbose) {
    AVENGER_OPTIONS['verbose'] = verbose ?? false;
    // Call with renderModule function from 'vega-scenegraph'
    renderModule('avenger', {
        handler: AvengerHandler,
        renderer: AvengerRenderer
    });
}

export async function viewToPng(view) {
    let {
        vegaSceneGroups,
        width,
        height,
        origin
    } = await view.runAsync().then(() => {
        try {
            // Workaround for https://github.com/vega/vega/issues/3481
            view.signal("geo_interval_init_tick", {});
        } catch (e) {
            // No geo_interval_init_tick signal
        }
    }).then(() => {
        return view.runAsync().then(
            () => {
                let padding = view.padding();
                return {
                    width: Math.max(0, view._viewWidth + padding.left + padding.right),
                    height: Math.max(0, view._viewHeight + padding.top + padding.bottom),
                    origin: [
                        padding.left + view._origin[0],
                        padding.top + view._origin[1]
                    ],
                    vegaSceneGroups: view.scenegraph().root
                }
            }
        );
    });

    const sceneGraph = await importScenegraph(vegaSceneGroups, width, height, origin);
    const png = await scene_graph_to_png(sceneGraph);
    return png;
}
