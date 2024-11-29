import {TextMark} from "../../lib/avenger_vega_renderer.js";
import {encodeSimpleArray} from "./util.js";

/**
 * @typedef {Object} TextItem
 * @property {string} text
 * @property {string} font
 * @property {number} fontSize
 * @property {string} fill
 * @property {number} x
 * @property {number} y
 * @property {number} angle
 * @property {number} radius
 * @property {number} theta
 * @property {number} dx
 * @property {number} dy
 * @property {number} limit
 * @property {number} opacity
 * @property {number} fillOpacity
 * @property {"alphabetic"|"top"|"middle"|"bottom"|"line-top"|"line-bottom"} baseline
 * @property {"left"|"center"|"right"} align
 * @property {number|"normal"|"bold"} fontWeight
 * @property {number} zindex
 */

/**
 * @typedef {Object} TextMarkSpec
 * @property {"text"} marktype
 * @property {boolean} clip
 * @property {boolean} interactive
 * @property {TextItem[]} items
 * @property {string} name
 * @property {string} role
 * @property {number} zindex
 */

/**
 * @param {TextMarkSpec} vegaTextMark
 * @param {boolean} force_clip
 * @returns {TextMark}
 */
export function importText(vegaTextMark, force_clip) {
    const len = vegaTextMark.items.length;
    const textMark = new TextMark(
        len, vegaTextMark.clip || force_clip, vegaTextMark.name, vegaTextMark.zindex
    );

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

    const fill = new Array(len).fill("");;
    const fillOpacity = new Float32Array(len).fill(1);
    let anyFill = false;

    const baseline = new Array(len);
    let anyBaseline = false;

    const align = new Array(len);
    let anyAlign = false;

    const fontWeight = new Array(len);
    let anyFontWeight = false;

    const zindex = new Int32Array(len).fill(0);
    let anyZindex = false;

    const items = vegaTextMark.items;
    items.forEach((item, i) => {
        // Semi-required properties have been initialized
        if (item.x != null) {
            x[i] = item.x;
        }

        if (item.x != null) {
            y[i] = item.y;
        }

        if (item.radius != null && item.theta != null) {
            x[i] += item.radius * Math.cos(item.theta - Math.PI / 2.0);
            y[i] += item.radius * Math.sin(item.theta - Math.PI / 2.0);
        }

        if (item.dx != null) {
            x[i] += item.dx;
        }

        if (item.dy != null) {
            y[i] += item.dy;
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

        if (item.zindex != null) {
            zindex[i] = item.zindex;
            anyZindex ||= true;
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
        const encoded = encodeSimpleArray(fill);
        textMark.set_color(encoded.values, encoded.indices, fillOpacity);
    }
    if (anyFont) {
        const encoded = encodeSimpleArray(font);
        textMark.set_font(encoded.values, encoded.indices);
    }
    if (anyBaseline) {
        const encoded = encodeSimpleArray(baseline);
        textMark.set_baseline(encoded.values, encoded.indices);
    }
    if (anyAlign) {
        const encoded = encodeSimpleArray(align);
        textMark.set_align(encoded.values, encoded.indices);
    }
    if (anyFontWeight) {
        const encoded = encodeSimpleArray(fontWeight);
        textMark.set_font_weight(encoded.values, encoded.indices);
    }
    if (anyZindex) {
        textMark.set_zindex(zindex);
    }
    return textMark;
}
