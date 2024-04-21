import { SceneGraph } from "../../pkg/avenger_wasm.js";
import { importGroup } from "./group.js";

/**
 * @param {import("group").GroupMarkSpec} groupMark
 * @param {number} width
 * @param {number} height
 * @param {[number, number]} origin
 * @returns {SceneGraph}
 */
export function importScenegraph(groupMark, width, height, origin) {
    const sceneGraph = new SceneGraph(width, height, origin[0], origin[1]);
    for (const vegaGroup of groupMark.items) {
        sceneGraph.add_group(importGroup(vegaGroup, groupMark.name));
    }
    return sceneGraph;
}