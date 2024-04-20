import {GroupMark} from "../../pkg/avenger_wasm.js";
import { importSymbol } from "./symbol.js"
import { importRule } from "./rule.js";
import {importText} from "./text.js";

export function importGroup(vegaGroup) {
    const groupMark = new GroupMark(
        vegaGroup.x, vegaGroup.y, vegaGroup.name, vegaGroup.width, vegaGroup.height
    );

    for (const vegaMark of vegaGroup.items) {
        switch (vegaMark.marktype) {
            case "symbol":
                groupMark.add_symbol_mark(importSymbol(vegaMark));
                break;
            case "rule":
                groupMark.add_rule_mark(importRule(vegaMark));
                break;
            case "text":
                groupMark.add_text_mark(importText(vegaMark));
                break;
            case "group":
                for (const groupItem of vegaMark.items) {
                    groupMark.add_group_mark(importGroup(groupItem));
                }
                break;
            default:
                console.log("Unsupported mark type: " + vegaMark.marktype)
        }
    }

    return groupMark;
}
