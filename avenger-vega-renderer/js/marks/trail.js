import {TrailMark} from "../../pkg/avenger_wasm.js";


/**
 * @typedef {Object} TrailItem
 * @property {number} x
 * @property {number} y
 * @property {number} size
 * @property {number} defined
 * @property {string|object} fill
 * @property {number} opacity
 * @property {number} fillOpacity
 */

/**
 * @typedef {Object} TrailMarkSpec
 * @property {"trail"} marktype
 * @property {boolean} clip
 * @property {boolean} interactive
 * @property {TrailItem[]} items
 * @property {string} name
 * @property {string} role
 * @property {number} zindex
 */

/**
 * @param {TrailMarkSpec} vegaLineMark
 * @param {boolean} force_clip
 * @returns {TrailMark}
 */
export function importTrail(vegaLineMark, force_clip) {
    const items = vegaLineMark.items;
    const len = items.length;

    const trailMark = new TrailMark(
        len, vegaLineMark.clip || force_clip, vegaLineMark.name, vegaLineMark.zindex
    );

    // Handle empty mark
    if (len === 0) {
        return trailMark;
    }

    // Set scalar values based on first element
    const firstItem = items[0];
    const fillOpacity = (firstItem.fillOpacity ?? 1) * (firstItem.opacity ?? 1);

    // Note: Vega calls the color fill, avenger calls it stroke
    trailMark.set_stroke(firstItem.fill, fillOpacity);

    // Semi-required values get initialized
    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);
    const size = new Float32Array(len).fill(1);
    let anySize = false;

    const defined = new Uint8Array(len).fill(1);
    let anyDefined = false;

    items.forEach((item, i) => {
        x[i] = item.x ?? 0;
        y[i] = item.y ?? 0;

        if (item.size != null) {
            size[i] = item.size;
            anySize ||= true;
        }

        if (item.defined != null) {
            defined[i] = item.defined;
            anyDefined ||= true;
        }
    })

    trailMark.set_xy(x, y);
    if (anySize) {
        trailMark.set_size(size);
    }
    if (anyDefined) {
        trailMark.set_defined(defined);
    }
    return trailMark;
}
