import {TextMark} from "../../pkg/avenger_wasm.js";
import {encodeStringArray} from "./util.js";

export function importText(vegaTextMark) {
    const len = vegaTextMark.items.length;
    const textMark = new TextMark(len, vegaTextMark.clip, vegaTextMark.name);

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

    const fill = new Array(len);
    const fillOpacity = new Float32Array(len).fill(1);
    let anyFill = false;

    const baseline = new Array(len);
    let anyBaseline = false;

    const align = new Array(len);
    let anyAlign = false;

    const fontWeight = new Array(len);
    let anyFontWeight = false;

    const items = vegaTextMark.items;
    items.forEach((item, i) => {
        // Semi-required properties have been initialized
        if (item.x != null) {
            x[i] = item.x;
        }

        if (item.x != null) {
            y[i] = item.y;
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
        const encoded = encodeStringArray(fill);
        textMark.set_color(encoded.values, encoded.indices, fillOpacity);
    }
    if (anyFont) {
        const encoded = encodeStringArray(font);
        textMark.set_font(encoded.values, encoded.indices);
    }
    if (anyBaseline) {
        const encoded = encodeStringArray(baseline);
        textMark.set_baseline(encoded.values, encoded.indices);
    }
    if (anyAlign) {
        const encoded = encodeStringArray(align);
        textMark.set_align(encoded.values, encoded.indices);
    }
    if (anyFontWeight) {
        const encoded = encodeStringArray(fontWeight);
        textMark.set_font_weight(encoded.values, encoded.indices);
    }

    return textMark;
}
