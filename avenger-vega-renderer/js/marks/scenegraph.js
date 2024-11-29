import { SceneGraph } from "../../lib/avenger_vega_renderer.js";
import { importGroup } from "./group.js";

/**
 * @typedef {Object} IResourceLoader
 * @property {function(): number} pending - Returns the number of pending load operations.
 * @property {function(string): Promise<Object>} sanitizeURL - Sanitizes a given URI and returns a promise that resolves to sanitized URI options.
 * @property {function(string): Promise<HTMLImageElement|Object>} loadImage - Attempts to load an image from a given URI, handling load counters, and returns a promise.
 * @property {function(): Promise<boolean>} ready - Returns a promise that resolves when all pending operations have completed.
 */

/**
 * @param {import("group").GroupMarkSpec} groupMark
 * @param {number} width
 * @param {number} height
 * @param {[number, number]} origin
 * @param {IResourceLoader} loader
 * @returns {Promise<SceneGraph>}
 */
export async function importScenegraph(groupMark, width, height, origin, loader) {
    const sceneGraph = new SceneGraph(width, height, origin[0], origin[1]);
    for (const vegaGroup of groupMark.items) {
        sceneGraph.add_group(await importGroup(vegaGroup, groupMark.name, false, loader));
    }
    return sceneGraph;
}
