import {GroupMark} from "../../pkg/avenger_wasm.js";
import { importSymbol } from "./symbol.js"
import { importRule } from "./rule.js";
import { importText } from "./text.js";
import { importRect } from "./rect.js";
import { importArc } from "./arc.js";
import { importPath } from "./path.js";
import { importShape } from "./shape.js";
import {importLine} from "./line.js";
import {importArea} from "./area.js";

/**
 * @typedef {import('./symbol.js').SymbolMarkSpec} SymbolMarkSpec
 * @typedef {import('./text.js').TextMarkSpec} TextMarkSpec
 * @typedef {import('./rule.js').RuleMarkSpec} RuleMarkSpec
 * @typedef {import('./rect.js').RectMarkSpec} RectMarkSpec
 * @typedef {import('./arc.js').ArcMarkSpec} ArcMarkSpec
 * @typedef {import('./path.js').PathMarkSpec} PathMarkSpec
 * @typedef {import('./shape.js').ShapeMarkSpec} ShapeMarkSpec
 * @typedef {import('./line.js').LineMarkSpec} LineMarkSpec
 * @typedef {import('./area.js').AreaMarkSpec} AreaMarkSpec
 *
 * @typedef {Object} GroupItemSpec
 * @property {"group"} marktype
 * @property {(GroupMarkSpec|SymbolMarkSpec|TextMarkSpec|RuleMarkSpec|RectMarkSpec|ArcMarkSpec|PathMarkSpec|ShapeMarkSpec|LineMarkSpec|AreaMarkSpec)[]} items
 * @property {number} x
 * @property {number} y
 * @property {number} width
 * @property {number} height
 * @property {number} x2
 * @property {number} y2
 * @property {boolean} clip
 * @property {string|object} fill
 * @property {string|object} stroke
 * @property {number} strokeWidth
 * @property {number} opacity
 * @property {number} fillOpacity
 * @property {number} strokeOpacity
 * @property {number} cornerRadius
 * @property {number} cornerRadiusTopLeft
 * @property {number} cornerRadiusTopRight
 * @property {number} cornerRadiusBottomLeft
 * @property {number} cornerRadiusBottomRight
 */

/**
 * @typedef {Object} GroupMarkSpec
 * @property {"group"} marktype
 * @property {boolean} interactive
 * @property {GroupItemSpec[]} items
 * @property {string} name
 * @property {string} role
 * @property {number} zindex
 */

/**
 * @param {GroupItemSpec} vegaGroup
 * @param {string} name
 * @param {boolean} forceClip
 * @returns {GroupMark}
 */
export function importGroup(vegaGroup, name, forceClip) {

    const width = vegaGroup.width ?? (vegaGroup.x2 != null? vegaGroup.x2 - vegaGroup.x: null);
    const height = vegaGroup.height ?? (vegaGroup.y2 != null? vegaGroup.y2 - vegaGroup.y: null);

    const groupMark = new GroupMark(
        vegaGroup.x ?? 0, vegaGroup.y ?? 0, name, width, height
    );

    for (const vegaMark of vegaGroup.items) {
        const clip = vegaGroup.clip || forceClip;
        switch (vegaMark.marktype) {
            case "symbol":
                groupMark.add_symbol_mark(importSymbol(vegaMark, clip));
                break;
            case "rule":
                groupMark.add_rule_mark(importRule(vegaMark, clip));
                break;
            case "rect":
                groupMark.add_rect_mark(importRect(vegaMark, clip));
                break;
            case "arc":
                groupMark.add_arc_mark(importArc(vegaMark, clip));
                break;
            case "path":
                groupMark.add_path_mark(importPath(vegaMark, clip));
                break;
            case "shape":
                groupMark.add_path_mark(importShape(vegaMark, clip));
                break;
            case "line":
                groupMark.add_line_mark(importLine(vegaMark, clip));
                break;
            case "area":
                groupMark.add_area_mark(importArea(vegaMark, clip));
                break;
            case "text":
                groupMark.add_text_mark(importText(vegaMark, clip));
                break;
            case "group":
                for (const groupItem of vegaMark.items) {
                    groupMark.add_group_mark(importGroup(groupItem, vegaMark.name, clip));
                }
                break;
        }
    }

    // Set styling
    const fillOpacity = (vegaGroup.opacity ?? 1) * (vegaGroup.fillOpacity ?? 1);
    if (typeof vegaGroup.fill === "string") {
        groupMark.set_fill(vegaGroup.fill, fillOpacity);
    } else if (vegaGroup.fill != null) {
        groupMark.set_fill_gradient(vegaGroup.fill, fillOpacity);
    }

    const strokeOpacity = (vegaGroup.opacity ?? 1) * (vegaGroup.strokeOpacity ?? 1);
    if (typeof vegaGroup.stroke === "string") {
        groupMark.set_stroke(vegaGroup.stroke, strokeOpacity);
    } else if (vegaGroup.stroke != null) {
        groupMark.set_stroke_gradient(vegaGroup.stroke, strokeOpacity);
    }

    groupMark.set_stroke_width(vegaGroup.strokeWidth);

    // set clip
    groupMark.set_clip(
        width,
        height,
        vegaGroup.cornerRadius,
        vegaGroup.cornerRadiusTopLeft,
        vegaGroup.cornerRadiusTopRight,
        vegaGroup.cornerRadiusBottomLeft,
        vegaGroup.cornerRadiusBottomRight,
    )

    return groupMark;
}
