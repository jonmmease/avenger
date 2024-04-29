import { AreaMark } from "../../lib/avenger_vega_renderer.generated.js";

/**
 * @typedef {Object} AreaItem
 * @property {number} strokeWidth
 * @property {"vertical"|"horizontal"} orient
 * @property {string|object} stroke
 * @property {string|object} fill
 * @property {"butt"|"round"|"square"} strokeCap
 * @property {"bevel"|"miter"|"round"} strokeJoin
 * @property {string|number[]} strokeDash
 * @property {number} x
 * @property {number} y
 * @property {number} x2
 * @property {number} y2
 * @property {number} defined
 * @property {number} opacity
 * @property {number} strokeOpacity
 * @property {number} fillOpacity
 */

/**
 * @typedef {Object} AreaMarkSpec
 * @property {"area"} marktype
 * @property {boolean} clip
 * @property {boolean} interactive
 * @property {AreaItem[]} items
 * @property {string} name
 * @property {string} role
 * @property {number} zindex
 */

/**
 * @param {AreaMarkSpec} vegaLineMark
 * @param {boolean} force_clip
 * @returns {AreaMark}
 */
export function importArea(vegaLineMark, force_clip) {
    const items = vegaLineMark.items;
    const len = items.length;

    const areaMark = new AreaMark(
        len, vegaLineMark.clip || force_clip, vegaLineMark.name, vegaLineMark.zindex
    );

    // Handle empty mark
    if (len === 0) {
        return areaMark;
    }

    // Set scalar values based on first element
    const firstItem = items[0];
    areaMark.set_stroke_width(firstItem.strokeWidth ?? 1);
    areaMark.set_stroke_join(firstItem.strokeJoin ?? "miter");
    areaMark.set_stroke_cap(firstItem.strokeCap ?? "butt");
    if (firstItem.strokeDash != null) {
        areaMark.set_stroke_dash(firstItem.strokeDash);
    }
    const strokeOpacity = (firstItem.strokeOpacity ?? 1) * (firstItem.opacity ?? 1);
    areaMark.set_stroke(firstItem.stroke, strokeOpacity);

    if (firstItem.fill != null) {
        const fillOpacity = (firstItem.fillOpacity ?? 1) * (firstItem.opacity ?? 1);
        areaMark.set_fill(firstItem.fill, fillOpacity);
    }

    if (firstItem.orient != null) {
        areaMark.set_orient(firstItem.orient)
    }

    // Semi-required values get initialized
    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);

    const x2 = new Float32Array(len).fill(0);
    let anyX2 = false;

    const y2 = new Float32Array(len).fill(0);
    let anyY2 = false;

    const defined = new Uint8Array(len).fill(1);
    let anyDefined = false;

    items.forEach((item, i) => {
        x[i] = item.x ?? 0;
        y[i] = item.y ?? 0;

        if (item.x2 != null) {
            x2[i] = item.x2;
            anyX2 ||= true;
        }

        if (item.y2 != null) {
            y2[i] = item.y2;
            anyY2 ||= true;
        }

        if (item.defined != null) {
            defined[i] = item.defined;
            anyDefined ||= true;
        }
    })

    areaMark.set_xy(x, y);

    if (anyX2) {
        areaMark.set_x2(x2);
    }

    if (anyY2) {
        areaMark.set_y2(y2);
    }

    if (anyDefined) {
        areaMark.set_defined(defined);
    }
    return areaMark;
}
