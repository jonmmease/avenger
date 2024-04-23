import {RectMark} from "../../pkg/avenger_wasm.js";
import {encodeSimpleArray} from "./util.js";


/**
 * Represents the style and configuration of a graphic element.
 * @typedef {Object} RectItem
 * @property {number} strokeWidth
 * @property {string|object} stroke
 * @property {string|number[]} strokeDash
 * @property {string|object} fill
 * @property {number} x
 * @property {number} y
 * @property {number} width
 * @property {number} height
 * @property {number} x2
 * @property {number} y2
 * @property {number} cornerRadius
 * @property {number} opacity
 * @property {number} fillOpacity
 * @property {number} strokeOpacity
 * @property {string} strokeCap
 * @property {number} zindex
 */

/**
 * Represents a graphical object configuration.
 * @typedef {Object} RectMarkSpec
 * @property {"rect"} marktype
 * @property {boolean} clip
 * @property {RectItem[]} items
 * @property {string} name
 * @property {number} zindex
 */

/**
 * @param {RectMarkSpec} vegaRectMark
 * @param {boolean} forceClip
 * @returns {RectMark}
 */
export function importRect(vegaRectMark, forceClip) {
    const items = vegaRectMark.items;
    const len = items.length;

    const rectMark = new RectMark(
        len, vegaRectMark.clip || forceClip, vegaRectMark.name, vegaRectMark.zindex
    );
    if (len === 0) {
        return rectMark;
    }

    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);
    const width = new Float32Array(len).fill(0);
    const height = new Float32Array(len).fill(0);

    const fill = new Array(len).fill("");
    let anyFill = false;
    let anyFillIsGradient = false;

    const stroke = new Array(len).fill("");
    let anyStroke = false;
    let anyStrokeIsGradient = false;

    const strokeWidth = new Float32Array(len);
    let anyStrokeWidth = false;

    const strokeOpacity = new Float32Array(len).fill(1);
    const fillOpacity = new Float32Array(len).fill(1);

    const cornerRadius = new Float32Array(len);
    let anyCornerRadius = false;

    const zindex = new Int32Array(len).fill(0);
    let anyZindex = false;

    items.forEach((item, i) => {
        if (item.x != null) {
            x[i] = item.x;
        }
        if (item.y != null) {
            y[i] = item.y;
        }
        if (item.width != null) {
            width[i] = item.width;
        } else if (item.x2 != null) {
            width[i] = item.x2 - x[i];
        }
        if (item.height != null) {
            height[i] = item.height;
        } else if (item.y2 != null) {
            height[i] = item.y2 - y[i];
        }
        if (item.fill != null) {
            fill[i] = item.fill;
            anyFill ||= true;
            anyFillIsGradient ||= typeof item.fill === "object";
        }
        fillOpacity[i] = (item.fillOpacity ?? 1) * (item.opacity ?? 1);

        if (item.stroke != null) {
            stroke[i] = item.stroke;
            anyStroke ||= true;
            anyStrokeIsGradient ||= typeof item.stroke === "object";
        }
        if (item.strokeWidth != null) {
            strokeWidth[i] = item.strokeWidth;
            anyStrokeWidth ||= true;
        }
        strokeOpacity[i] = (item.strokeOpacity ?? 1) * (item.opacity ?? 1);

        if (item.cornerRadius != null) {
            cornerRadius[i] = item.cornerRadius;
            anyCornerRadius ||= true;
        }

        if (item.zindex != null) {
            zindex[i] = item.zindex;
            anyZindex ||= true;
        }
    })

    rectMark.set_xy(x, y);
    rectMark.set_width(width);
    rectMark.set_height(height);

    if (anyFill) {
        if (anyFillIsGradient) {
            rectMark.set_fill_gradient(fill, fillOpacity);
        } else {
            const encoded = encodeSimpleArray(fill);
            rectMark.set_fill(encoded.values, encoded.indices, strokeOpacity);
        }
    }

    if (anyStroke) {
        if (anyStrokeIsGradient) {
            rectMark.set_stroke_gradient(stroke, strokeOpacity);
        } else {
            const encoded = encodeSimpleArray(stroke);
            rectMark.set_stroke(encoded.values, encoded.indices, strokeOpacity);
        }
    }

    if (anyStrokeWidth) {
        rectMark.set_stroke_width(strokeWidth);
    }


    if (anyCornerRadius) {
        rectMark.set_corner_radius(cornerRadius);
    }

    if (anyZindex) {
        rectMark.set_zindex(zindex);
    }

    return rectMark;
}
