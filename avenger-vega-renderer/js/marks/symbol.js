import {SymbolMark} from "../../pkg/avenger_wasm.js";
import {encodeSimpleArray} from "./util.js";


/**
 * @typedef {Object} SymbolItem
 * @property {number} strokeWidth
 * @property {string|object} fill
 * @property {string|object} stroke
 * @property {number} x
 * @property {number} y
 * @property {number} size
 * @property {number} opacity
 * @property {number} strokeOpacity
 * @property {number} fillOpacity
 * @property {string} shape
 * @property {number} angle
 * @property {number} zindex
 */

/**
 * @typedef {Object} SymbolMarkSpec
 * @property {"symbol"} marktype
 * @property {boolean} clip
 * @property {boolean} interactive
 * @property {SymbolItem[]} items
 * @property {string} name
 * @property {string} role
 * @property {number} zindex
 */

/**
 * @param {SymbolMarkSpec} vegaSymbolMark
 * @param {boolean} force_clip
 * @returns {SymbolMark}
 */
export function importSymbol(vegaSymbolMark, force_clip) {
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

    const zindex = new Int32Array(len).fill(0);
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
            const encoded = encodeSimpleArray(fill);
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
            const encoded = encodeSimpleArray(stroke);
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
        const encoded = encodeSimpleArray(shapes);
        console.log()
        symbolMark.set_shape(encoded.values, encoded.indices);
    }

    return symbolMark;
}
