import {LineMark} from "../../pkg/avenger_wasm.js";


/**
 * @typedef {Object} LineItem
 * @property {number} strokeWidth
 * @property {string|object} stroke
 * @property {"butt"|"round"|"square"} strokeCap
 * @property {"bevel"|"miter"|"round"} strokeJoin
 * @property {string|number[]} strokeDash
 * @property {number} x
 * @property {number} y
 * @property {number} defined
 * @property {number} opacity
 * @property {number} strokeOpacity
 */

/**
 * @typedef {Object} LineMarkSpec
 * @property {"line"} marktype
 * @property {boolean} clip
 * @property {boolean} interactive
 * @property {LineItem[]} items
 * @property {string} name
 * @property {string} role
 * @property {number} zindex
 */

/**
 * @param {LineMarkSpec} vegaLineMark
 * @param {boolean} force_clip
 * @returns {LineMark}
 */
export function importLine(vegaLineMark, force_clip) {
    const items = vegaLineMark.items;
    const len = items.length;

    const lineMark = new LineMark(
        len, vegaLineMark.clip || force_clip, vegaLineMark.name, vegaLineMark.zindex
    );

    // Handle empty mark
    if (len === 0) {
        return lineMark;
    }

    // Set scalar values based on first element
    const firstItem = items[0];
    lineMark.set_stroke_width(firstItem.strokeWidth ?? 1);
    lineMark.set_stroke_join(firstItem.strokeJoin ?? "miter");
    lineMark.set_stroke_cap(firstItem.strokeCap ?? "butt");
    if (firstItem.strokeDash != null) {
        lineMark.set_stroke_dash(firstItem.strokeDash);
    }
    const strokeOpacity = (firstItem.strokeOpacity ?? 1) * (firstItem.opacity ?? 1);
    lineMark.set_stroke(firstItem.stroke, strokeOpacity);

    // Semi-required values get initialized
    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);
    const defined = new Uint8Array(len).fill(1);
    let anyDefined = false;

    items.forEach((item, i) => {
        x[i] = item.x ?? 0;
        y[i] = item.y ?? 0;

        if (item.defined != null) {
            defined[i] = item.defined;
            anyDefined ||= true;
        }
    })

    lineMark.set_xy(x, y);
    if (anyDefined) {
        lineMark.set_defined(defined);
    }
    return lineMark;
}
