import { AvengerCanvas, SceneGraph, GroupMark, SymbolMark, RuleMark, TextMark, scene_graph_to_png } from "../pkg/avenger_wasm.js";
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
        this._avengerHtmlCanvas.style.width = width + "px";
        this._avengerHtmlCanvas.style.height = height + "px";

        // Add event canvas to top element
        this._handlerCanvas = document.createElement('canvas');
        domClear(topEl, 0).appendChild(this._handlerCanvas);
        this._handlerCanvas.setAttribute('class', 'marks');

        // Create Avenger canvas
        console.log("create: ", width, height, origin);
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
        console.log("scene graph construction time: " + (performance.now() - this._lastRenderFinishTime));
        this._avengerCanvasPromise.then((avengerCanvas) => {
            var start = performance.now();
            const sceneGraph = importScenegraph(
                scene,
                avengerCanvas.width(),
                avengerCanvas.height(),
                [avengerCanvas.origin_x(), avengerCanvas.origin_y()]
            );
            avengerCanvas.set_scene(sceneGraph);
            console.log("_render time: " + (performance.now() - start));
        });
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
    const sceneGraph = new SceneGraph(width, height, origin[0], origin[1]);
    for (const vegaGroup of vegaSceneGroups.items) {
        sceneGraph.add_group(importGroup(vegaGroup));
    }
    return sceneGraph;
}

function importGroup(vegaGroup) {
    const groupMark = new GroupMark(
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
            case "text":
                groupMark.add_text_mark(importText(vegaMark));
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

function importSymbol(vegaSymbolMark, force_clip) {
    const items = vegaSymbolMark.items;
    const len = items.length;

    const symbolMark = new SymbolMark(
        len, vegaSymbolMark.clip || force_clip, vegaSymbolMark.name, vegaSymbolMark.zindex
    );

    // Handle empty mark
    if (len === 0) {
        return symbolMark;
    }

    const firstItem = items[0];
    const firstShape = firstItem.shape ?? "circle";

    if (firstShape === "stroke") {
        // TODO: Handle line legends
        return symbolMark
    }

    // Only include stroke_width if there is a stroke color
    const firstHasStroke = firstItem.stroke != null;
    let strokeWidth;
    if (firstHasStroke) {
        strokeWidth = firstItem.strokeWidth ?? 1;
    }
    symbolMark.set_stroke_width(strokeWidth);

    // Semi-required values get initialized
    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);

    const fill = new Array(len);
    let anyFill = false;
    let fillIsGradient = firstItem.fill != null && typeof firstItem.fill === "object";

    const size = new Float32Array(len).fill(20);
    let anySize = false;

    const stroke = new Array(len);
    let anyStroke = false;
    let strokeIsGradient = firstItem.stroke != null && typeof firstItem.stroke === "object";

    const angle = new Float32Array(len).fill(0);
    let anyAngle = false;

    const zindex = new Float32Array(len).fill(0);
    let anyZindex = false;

    const fillOpacity = new Float32Array(len).fill(1);
    const strokeOpacity = new Float32Array(len).fill(1);

    const shapes = new Array(len);
    let anyShape = false;

    items.forEach((item, i) => {
        x[i] = item.x ?? 0;
        y[i] = item.y ?? 0;

        const baseOpacity = item.opacity ?? 1;
        fillOpacity[i] = (item.fillOpacity ?? 1) * baseOpacity;
        strokeOpacity[i] = (item.strokeOpacity ?? 1) * baseOpacity;

        if (item.fill != null) {
            fill[i] = item.fill;
            anyFill ||= true;
        }

        if (item.size != null) {
            size[i] = item.size;
            anySize = true;
        }

        if (item.stroke != null) {
            stroke[i] = item.stroke;
            anyStroke ||= true;
        }

        if (item.angle != null) {
            angle[i] = item.angle;
            anyAngle ||= true;
        }

        if (item.zindex != null) {
            zindex[i] = item.zindex;
            anyZindex ||= true;
        }

        if (item.shape != null) {
            shapes[i] = item.shape;
            anyShape ||= true;
        }
    })

    symbolMark.set_xy(x, y);

    if (anyFill) {
        if (fillIsGradient) {
            symbolMark.set_fill_gradient(fill, fillOpacity);
        } else {
            const encoded = encodeStringArray(fill);
            symbolMark.set_fill(encoded.values, encoded.indices, fillOpacity);
        }
    }

    if (anySize) {
        symbolMark.set_size(size);
    }

    if (anyStroke) {
        if (strokeIsGradient) {
            symbolMark.set_stroke_gradient(stroke, strokeOpacity);
        } else {
            const encoded = encodeStringArray(stroke);
            symbolMark.set_stroke(encoded.values, encoded.indices, strokeOpacity);
        }
    }

    if (anyAngle) {
        symbolMark.set_angle(angle);
    }

    if (anyZindex) {
        symbolMark.set_zindex(zindex);
    }

    if (anyShape) {
        const encoded = encodeStringArray(shapes);
        console.log()
        symbolMark.set_shape(encoded.values, encoded.indices);
    }

    return symbolMark;
}

function importRule(vegaRuleMark, forceClip) {
    const items = vegaRuleMark.items;
    const len = items.length;

    const ruleMark = new RuleMark(
        len, vegaRuleMark.clip || forceClip, vegaRuleMark.name, vegaRuleMark.zindex
    );
    if (len === 0) {
        return ruleMark;
    }

    const firstItem = items[0];

    const x0 = new Float32Array(len).fill(0);
    const y0 = new Float32Array(len).fill(0);
    const x1 = new Float32Array(len).fill(0);
    const y1 = new Float32Array(len).fill(0);

    const strokeWidth = new Float32Array(len);
    let anyStrokeWidth = false;

    const strokeOpacity = new Float32Array(len).fill(1);

    const stroke = new Array(len);
    let anyStroke = false;
    let strokeIsGradient = firstItem.stroke != null && typeof firstItem.stroke === "object";

    const strokeCap = new Array(len);
    let anyStrokeCap = false;

    const strokeDash = new Array(len);
    let anyStrokeDash = false;

    const zindex = new Float32Array(len).fill(0);
    let anyZindex = false;

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
        if (item.strokeWidth != null) {
            strokeWidth[i] = item.strokeWidth;
            anyStrokeWidth ||= true;
        }
        strokeOpacity[i] = (item.strokeOpacity ?? 1) * (item.opacity ?? 1);

        if (item.stroke != null) {
            stroke[i] = item.stroke;
            anyStroke ||= true;
        }

        if (item.strokeCap != null) {
            strokeCap[i] = item.strokeCap;
            anyStrokeCap ||= true;
        }

        if (item.strokeDash != null) {
            strokeDash[i] = item.strokeDash;
            anyStrokeDash ||= true;
        }

        if (item.zindex != null) {
            zindex[i] = item.zindex;
            anyZindex ||= true;
        }
    })

    ruleMark.set_xy(x0, y0, x1, y1);

    if (anyStrokeWidth) {
        ruleMark.set_stroke_width(strokeWidth);
    }

    if (anyStroke) {
        if (strokeIsGradient) {
            ruleMark.set_stroke_gradient(stroke, strokeOpacity);
        } else {
            const encoded = encodeStringArray(stroke);
            ruleMark.set_stroke(encoded.values, encoded.indices, strokeOpacity);
        }
    }

    if (anyStrokeCap) {
        ruleMark.set_stroke_cap(strokeCap);
    }

    if (anyStrokeDash) {
        ruleMark.set_stroke_dash(strokeDash);
    }

    if (anyZindex) {
        ruleMark.set_zindex(zindex);
    }

    return ruleMark;
}

function importText(vegaTextMark) {
    const len = vegaTextMark.items.length;
    const textMark = new TextMark(len, vegaTextMark.clip, vegaTextMark.name);

    // semi-required columns where we will pass the full array no matter what
    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);
    const text = new Array(len).fill("");

    // Optional properties where we will only pass the full array if any value is specified
    const fontSize = new Float32Array(len);
    let anyFontSize = false;

    const angle = new Float32Array(len);
    let anyAngle = false;

    const limit = new Float32Array(len);
    let anyLimit = false;

    // String properties that typically have lots of duplicates, so
    // unique values and indices.
    const font = new Array(len);
    let anyFont = false;

    const fill = new Array(len);
    const fillOpacity = new Float32Array(len).fill(1);
    let anyFill = false;

    const baseline = new Array(len);
    let anyBaseline = false;

    const align = new Array(len);
    let anyAlign = false;

    const fontWeight = new Array(len);
    let anyFontWeight = false;

    const items = vegaTextMark.items;
    items.forEach((item, i) => {
        // Semi-required properties have been initialized
        if (item.x != null) {
            x[i] = item.x;
        }

        if (item.x != null) {
            y[i] = item.y;
        }

        if (item.text != null) {
            text[i] = item.text;
        }

        // Optional properties have not been initialized, we need to track if any are specified
        if (item.fontSize != null) {
            fontSize[i] = item.fontSize;
            anyFontSize ||= true;
        }

        if (item.angle != null) {
            angle[i] = item.angle;
            anyAngle ||= true;
        }

        if (item.limit != null) {
            limit[i] = item.limit;
            anyLimit ||= true;
        }

        if (item.fill != null) {
            fill[i] = item.fill;
            anyFill ||= true;
        }
        fillOpacity[i] = (item.fillOpacity ?? 1) * (item.opacity ?? 1);

        if (item.font != null) {
            font[i] = item.font;
            anyFont ||= true;
        }

        if (item.baseline != null) {
            baseline[i] = item.baseline;
            anyBaseline ||= true;
        }

        if (item.align != null) {
            align[i] = item.align;
            anyAlign ||= true;
        }

        if (item.fontWeight != null) {
            fontWeight[i] = item.fontWeight;
            anyFontWeight ||= true;
        }
    })

    // Set semi-required properties as full arrays
    textMark.set_xy(x, y);
    textMark.set_text(text);

    // Set optional properties if any were defined
    if (anyFontSize) {
        textMark.set_font_size(fontSize)
    }
    if (anyAngle) {
        textMark.set_angle(angle);
    }
    if (anyLimit) {
        textMark.set_font_limit(limit);
    }

    // String columns to pass as encoded unique values + indices
    if (anyFill) {
        const encoded = encodeStringArray(fill);
        textMark.set_color(encoded.values, encoded.indices, fillOpacity);
    }
    if (anyFont) {
        const encoded = encodeStringArray(font);
        textMark.set_font(encoded.values, encoded.indices);
    }
    if (anyBaseline) {
        const encoded = encodeStringArray(baseline);
        textMark.set_baseline(encoded.values, encoded.indices);
    }
    if (anyAlign) {
        const encoded = encodeStringArray(align);
        textMark.set_align(encoded.values, encoded.indices);
    }
    if (anyFontWeight) {
        const encoded = encodeStringArray(fontWeight);
        textMark.set_font_weight(encoded.values, encoded.indices);
    }

    return textMark;
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
        values: uniqueStringsArray,
        indices,
    };
}

export function registerVegaRenderer(renderModule) {
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

    const sceneGraph = importScenegraph(vegaSceneGroups, width, height, origin);
    const png = await scene_graph_to_png(sceneGraph);
    return png;
}
