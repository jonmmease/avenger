import {SymbolMark, GroupMark, LineMark} from "../../pkg/avenger_wasm.js";
import {encodeSimpleArray} from "./util.js";


/**
 * @typedef {Object} SymbolItem
 * @property {number} strokeWidth
 * @property {"butt"|"round"|"square"} strokeCap
 * @property {"bevel"|"miter"|"round"} strokeJoin
 * @property {string|number[]} strokeDash
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

    const fill = new Array(len).fill("");;
    let anyFill = false;
    let anyFillIsGradient = false;

    const size = new Float32Array(len).fill(20);
    let anySize = false;

    const stroke = new Array(len).fill("");;
    let anyStroke = false;
    let anyStrokeIsGradient = false;

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
            anyFillIsGradient ||= typeof item.fill === "object";
        }

        if (item.size != null) {
            size[i] = item.size;
            anySize = true;
        }

        if (item.stroke != null) {
            stroke[i] = item.stroke;
            anyStroke ||= true;
            anyStrokeIsGradient ||= typeof item.stroke === "object";
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
        if (anyFillIsGradient) {
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
        if (anyStrokeIsGradient) {
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
        symbolMark.set_shape(encoded.values, encoded.indices);
    }

    return symbolMark;
}


/**
 * Handle special case of symbols with shape == "stroke". This happens when lines are
 * sed in legends. We convert these to a group of regular line marks
 * @param {SymbolMarkSpec} vegaSymbolMark
 * @param {boolean} force_clip
 * @returns {GroupMark}
 */
export function importStrokeLegend(vegaSymbolMark, force_clip) {
    const groupMark = new GroupMark(
        0, 0, "symbol_line_legend", undefined, undefined
    );

    for (let item of vegaSymbolMark.items) {
        let width = Math.sqrt(item.size ?? 100.0);
        let x = item.x ?? 0;
        let y = item.y ?? 0;
        const lineMark = new LineMark(
            2, vegaSymbolMark.clip || force_clip, undefined, vegaSymbolMark.zindex
        );

        lineMark.set_xy(
            new Float32Array([x - width / 2.0, x + width / 2.0]),
            new Float32Array([y, y])
        )

        lineMark.set_stroke(item.stroke ?? "", (item.opacity ?? 1) * (item.strokeOpacity ?? 1));
        if (item.strokeWidth != null) {
            lineMark.set_stroke_width(item.strokeWidth);
        }

        if (item.strokeCap != null) {
            lineMark.set_stroke_cap(item.strokeCap);
        }

        if (item.strokeJoin != null) {
            lineMark.set_stroke_join(item.strokeJoin);
        }

        if (item.strokeDash != null) {
            lineMark.set_stroke_dash(item.strokeDash);
        }

        groupMark.add_line_mark(lineMark);
    }

    return groupMark
}
