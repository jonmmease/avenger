import {RuleMark} from "../../lib/avenger_vega_renderer.js";
import {encodeSimpleArray} from "./util.js";


/**
 * Represents the style and configuration of a graphic element.
 * @typedef {Object} RuleItem
 * @property {number} strokeWidth
 * @property {string} stroke
 * @property {string|number[]} strokeDash
 * @property {number} x
 * @property {number} x2
 * @property {number} y
 * @property {number} y2
 * @property {number} opacity
 * @property {number} strokeOpacity
 * @property {string} strokeCap
 * @property {number} zindex
 */

/**
 * Represents a graphical object configuration.
 * @typedef {Object} RuleMarkSpec
 * @property {"rule"} marktype
 * @property {boolean} clip
 * @property {RuleItem[]} items
 * @property {string} name
 * @property {number} zindex
 */

/**
 * @param {RuleMarkSpec} vegaRuleMark
 * @param {boolean} forceClip
 * @returns {RuleMark}
 */
export function importRule(vegaRuleMark, forceClip) {
    const items = vegaRuleMark.items;
    const len = items.length;

    const ruleMark = new RuleMark(
        len, vegaRuleMark.clip || forceClip, vegaRuleMark.name, vegaRuleMark.zindex
    );
    if (len === 0) {
        return ruleMark;
    }

    const x0 = new Float32Array(len).fill(0);
    const y0 = new Float32Array(len).fill(0);
    const x1 = new Float32Array(len).fill(0);
    const y1 = new Float32Array(len).fill(0);

    const strokeWidth = new Float32Array(len);
    let anyStrokeWidth = false;

    const strokeOpacity = new Float32Array(len).fill(1);

    const stroke = new Array(len).fill("");
    let anyStroke = false;
    let anyStrokeIsGradient = false;

    const strokeCap = new Array(len);
    let anyStrokeCap = false;

    const strokeDash = new Array(len);
    let anyStrokeDash = false;

    const zindex = new Int32Array(len).fill(0);
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
            anyStrokeIsGradient ||= typeof item.stroke === "object";
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
        if (anyStrokeIsGradient) {
            ruleMark.set_stroke_gradient(stroke, strokeOpacity);
        } else {
            const encoded = encodeSimpleArray(stroke);
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
