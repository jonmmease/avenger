import {PathMark} from "../../lib/avenger_vega_renderer.js";
import {encodeSimpleArray} from "./util.js";


/**
 @typedef {function(ShapeItem): string} ShapeFunction
 */

/**
 * @typedef {Object} ShapeItem
 * @property {string|object} fill
 * @property {string|object} stroke
 * @property {number} x
 * @property {number} y
 * @property {number} scaleX
 * @property {number} scaleY
 * @property {number} angle
 *
 * @property {number} strokeWidth
 * @property {string} strokeCap
 * @property {string} strokeJoin
 *
 * @property {number} opacity
 * @property {number} strokeOpacity
 * @property {number} fillOpacity
 * @property {ShapeFunction} shape
 * @property {number} zindex
 */

/**
 * @typedef {Object} ShapeMarkSpec
 * @property {"shape"} marktype
 * @property {boolean} clip
 * @property {boolean} interactive
 * @property {ShapeItem[]} items
 * @property {string} name
 * @property {string} role
 * @property {number} zindex
 */

/**
 * @param {ShapeMarkSpec} vegaShapeMark
 * @param {boolean} force_clip
 * @returns {PathMark}
 */
export function importShape(vegaShapeMark, force_clip) {
    const items = vegaShapeMark.items;
    const len = items.length;

    const pathMark = new PathMark(
        len, vegaShapeMark.clip || force_clip, vegaShapeMark.name, vegaShapeMark.zindex
    );

    // Handle empty mark
    if (len === 0) {
        return pathMark;
    }

    // Only include stroke_width if there is a stroke color
    const firstItem = items[0];
    const firstHasStroke = firstItem.stroke != null;
    let strokeWidth;
    if (firstHasStroke) {
        strokeWidth = firstItem.strokeWidth ?? 1;
    }
    pathMark.set_stroke_width(strokeWidth);

    // Semi-required values get initialized
    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);
    const scale_x = new Float32Array(len).fill(1);
    const scale_y = new Float32Array(len).fill(1);
    const angle = new Float32Array(len).fill(0);

    const fill = new Array(len).fill("");;
    let anyFill = false;
    let anyFillIsGradient = false;

    const stroke = new Array(len).fill("");;
    let anyStroke = false;
    let anyStrokeIsGradient = false;

    const zindex = new Int32Array(len).fill(0);
    let anyZindex = false;

    const fillOpacity = new Float32Array(len).fill(1);
    const strokeOpacity = new Float32Array(len).fill(1);

    const path = new Array(len).fill("");

    items.forEach((item, i) => {
        x[i] = item.x ?? 0;
        y[i] = item.y ?? 0;
        scale_x[i] = item.scaleX ?? 1;
        scale_y[i] = item.scaleY ?? 1;
        angle[i] = item.angle ?? 0;

        const baseOpacity = item.opacity ?? 1;
        fillOpacity[i] = (item.fillOpacity ?? 1) * baseOpacity;
        strokeOpacity[i] = (item.strokeOpacity ?? 1) * baseOpacity;

        if (item.fill != null) {
            fill[i] = item.fill;
            anyFill ||= true;
            anyFillIsGradient ||= typeof item.fill === "object";
        }

        if (item.stroke != null) {
            stroke[i] = item.stroke;
            anyStroke ||= true;
            anyStrokeIsGradient ||= typeof item.stroke === "object";
        }

        if (item.zindex != null) {
            zindex[i] = item.zindex;
            anyZindex ||= true;
        }

        if (item.shape != null) {
            // @ts-ignore
            item.shape.context();
            path[i] = item.shape(item) ?? "";
        }
    })

    pathMark.set_transform(x, y, scale_x, scale_y, angle);

    const encodedPaths = encodeSimpleArray(path);
    pathMark.set_path(encodedPaths.values, encodedPaths.indices);

    if (anyFill) {
        if (anyFillIsGradient) {
            pathMark.set_fill_gradient(fill, fillOpacity);
        } else {
            const encoded = encodeSimpleArray(fill);
            pathMark.set_fill(encoded.values, encoded.indices, fillOpacity);
        }
    }

    if (anyStroke) {
        if (anyStrokeIsGradient) {
            pathMark.set_stroke_gradient(stroke, strokeOpacity);
        } else {
            const encoded = encodeSimpleArray(stroke);
            pathMark.set_stroke(encoded.values, encoded.indices, strokeOpacity);
        }
    }

    if (anyZindex) {
        pathMark.set_zindex(zindex);
    }

    return pathMark;
}
