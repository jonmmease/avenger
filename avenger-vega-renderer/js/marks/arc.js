import {ArcMark} from "../../pkg/avenger_wasm.js";
import {encodeSimpleArray} from "./util.js";


/**
 * Represents the style and configuration of a graphic element.
 * @typedef {Object} ArcItem
 * @property {number} x
 * @property {number} y
 * @property {number} startAngle
 * @property {number} endAngle
 * @property {number} outerRadius
 * @property {number} innerRadius
 * @property {string|object} fill
 * @property {string|object} stroke
 * @property {number} strokeWidth
 * @property {number} opacity
 * @property {number} fillOpacity
 * @property {number} strokeOpacity
 * @property {number} zindex
 */

/**
 * Represents a graphical object configuration.
 * @typedef {Object} ArcMarkSpec
 * @property {"arc"} marktype
 * @property {boolean} clip
 * @property {ArcItem[]} items
 * @property {string} name
 * @property {number} zindex
 */

/**
 * @param {ArcMarkSpec} vegaArcMark
 * @param {boolean} forceClip
 * @returns {ArcMark}
 */
export function importArc(vegaArcMark, forceClip) {
    const items = vegaArcMark.items;
    const len = items.length;

    const arcMark = new ArcMark(
        len, vegaArcMark.clip || forceClip, vegaArcMark.name, vegaArcMark.zindex
    );
    if (len === 0) {
        return arcMark;
    }

    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);

    const startAngle = new Float32Array(len).fill(0);
    let anyStartAngle = false;

    const endAngle = new Float32Array(len).fill(0);
    let anyEndAngle = false;

    const outerRadius = new Float32Array(len).fill(0);
    let anyOuterRadius = false;

    const innerRadius = new Float32Array(len).fill(0);
    let anyInnerRadius = false;

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

    const zindex = new Int32Array(len).fill(0);
    let anyZindex = false;

    items.forEach((item, i) => {
        if (item.x != null) {
            x[i] = item.x;
        }
        if (item.y != null) {
            y[i] = item.y;
        }
        if (item.startAngle != null) {
            startAngle[i] = item.startAngle;
            anyStartAngle ||= true;
        }
        if (item.endAngle != null) {
            endAngle[i] = item.endAngle;
            anyEndAngle ||= true;
        }
        if (item.outerRadius != null) {
            outerRadius[i] = item.outerRadius;
            anyOuterRadius ||= true;
        }
        if (item.innerRadius != null) {
            innerRadius[i] = item.innerRadius;
            anyInnerRadius ||= true;
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

        if (item.zindex != null) {
            zindex[i] = item.zindex;
            anyZindex ||= true;
        }
    })

    arcMark.set_xy(x, y);
    if (anyStartAngle) {
        arcMark.set_start_angle(startAngle);
    }
    if (anyEndAngle) {
        arcMark.set_end_angle(endAngle);
    }
    if (anyOuterRadius) {
        arcMark.set_outer_radius(outerRadius);
    }
    if (anyInnerRadius) {
        arcMark.set_inner_radius(innerRadius)
    }

    if (anyFill) {
        if (anyFillIsGradient) {
            arcMark.set_fill_gradient(fill, fillOpacity);
        } else {
            const encoded = encodeSimpleArray(fill);
            arcMark.set_fill(encoded.values, encoded.indices, fillOpacity);
        }
    }

    if (anyStroke) {
        if (anyStrokeIsGradient) {
            arcMark.set_stroke_gradient(stroke, strokeOpacity);
        } else {
            const encoded = encodeSimpleArray(stroke);
            arcMark.set_stroke(encoded.values, encoded.indices, strokeOpacity);
        }
    }

    if (anyStrokeWidth) {
        arcMark.set_stroke_width(strokeWidth);
    }

    if (anyZindex) {
        arcMark.set_zindex(zindex);
    }

    return arcMark;
}
