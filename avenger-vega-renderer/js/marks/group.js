import {GroupMark} from "../../pkg/avenger_wasm.js";
import { importSymbol } from "./symbol.js"
import { importRule } from "./rule.js";
import {importText} from "./text.js";
import {importRect} from "./rect.js";

/**
 * @typedef {import('./symbol.js').SymbolMarkSpec} SymbolMarkSpec
 * @typedef {import('./text.js').TextMarkSpec} TextMarkSpec
 * @typedef {import('./rule.js').RuleMarkSpec} RuleMarkSpec
 * @typedef {import('./rect.js').RectMarkSpec} RectMarkSpec
 *
 *
 * @typedef {Object} GroupItemSpec
 * @property {"group"} marktype
 * @property {(GroupMarkSpec|SymbolMarkSpec|TextMarkSpec|RuleMarkSpec|RectMarkSpec)[]} items
 * @property {number} x
 * @property {number} y
 * @property {number} width
 * @property {number} height
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
 * @property {boolean} clip
 * @property {boolean} interactive
 * @property {GroupItemSpec[]} items
 * @property {string} name
 * @property {string} role
 * @property {number} zindex
 */

/**
 * @param {GroupItemSpec} vegaGroup
 * @param {string} name
 * @returns {GroupMark}
 */
export function importGroup(vegaGroup, name) {
    const groupMark = new GroupMark(
        vegaGroup.x, vegaGroup.y, name, vegaGroup.width, vegaGroup.height
    );

    const forceClip = false;
    for (const vegaMark of vegaGroup.items) {
        switch (vegaMark.marktype) {
            case "symbol":
                groupMark.add_symbol_mark(importSymbol(vegaMark, forceClip));
                break;
            case "rule":
                groupMark.add_rule_mark(importRule(vegaMark, forceClip));
                break;
            case "rect":
                groupMark.add_rect_mark(importRect(vegaMark, forceClip));
                break;
            case "text":
                groupMark.add_text_mark(importText(vegaMark, forceClip));
                break;
            case "group":
                for (const groupItem of vegaMark.items) {
                    groupMark.add_group_mark(importGroup(groupItem, vegaMark.name));
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

    // set clip
    groupMark.set_clip(
        vegaGroup.width,
        vegaGroup.height,
        vegaGroup.cornerRadius,
        vegaGroup.cornerRadiusTopLeft,
        vegaGroup.cornerRadiusTopRight,
        vegaGroup.cornerRadiusBottomLeft,
        vegaGroup.cornerRadiusBottomRight,
    )

    return groupMark;
}
