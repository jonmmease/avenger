import {GroupMark} from "../../pkg/avenger_wasm.js";
import { importSymbol } from "./symbol.js"
import { importRule } from "./rule.js";
import {importText} from "./text.js";

/**
 * @typedef {import('./symbol.js').SymbolMarkSpec} SymbolMarkSpec
 * @typedef {import('./text.js').TextMarkSpec} TextMarkSpec
 * @typedef {import('./rule.js').RuleMarkSpec} RuleMarkSpec
 *
 *
 * @typedef {Object} GroupItemSpec
 * @property {"group"} marktype
 * @property {boolean} clip
 * @property {(GroupMarkSpec|SymbolMarkSpec|TextMarkSpec|RuleMarkSpec)[]} items
 * @property {string} fill
 * @property {string} stroke
 * @property {number} x
 * @property {number} y
 * @property {number} width
 * @property {number} height
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

    return groupMark;
}
