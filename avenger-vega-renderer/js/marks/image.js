import {ImageMark} from "../../lib/avenger_vega_renderer.generated.js";
import {encodeSimpleArray} from "./util.js";


/**
 * Represents the style and configuration of a graphic element.
 * @typedef {Object} ImageItem
 * @property {string} url
 * @property {number} x
 * @property {number} y
 * @property {number} width
 * @property {number} height
 * @property {number} x2
 * @property {number} y2
 * @property {"left"|"center"|"right"} align
 * @property {"top"|"middle"|"bottom"} baseline
 * @property {boolean} smooth
 * @property {boolean} aspect
 * @property {number} zindex
 */

/**
 * Represents a graphical object configuration.
 * @typedef {Object} ImageMarkSpec
 * @property {"image"} marktype
 * @property {boolean} clip
 * @property {ImageItem[]} items
 * @property {string} name
 * @property {number} zindex
 */

/**
 * @typedef {import('./scenegraph.js').IResourceLoader} IResourceLoader
 *
 * @param {ImageMarkSpec} vegaImageMark
 * @param {boolean} forceClip
 * @param {IResourceLoader} loader
 * @returns {Promise<ImageMark>}
 */
export async function importImage(vegaImageMark, forceClip, loader) {
    const items = vegaImageMark.items;
    const len = items.length;

    const imageMark = new ImageMark(
        len, vegaImageMark.clip || forceClip, vegaImageMark.name, vegaImageMark.zindex
    );
    if (len === 0) {
        return imageMark;
    }

    // Set scalar properties based on first item
    const firstItem = items[0];
    if (firstItem.aspect != null) {
        imageMark.set_aspect(firstItem.aspect);
    }
    if (firstItem.smooth != null) {
        imageMark.set_smooth(firstItem.smooth);
    }

    const image = new Array(len);
    const x = new Float32Array(len).fill(0);
    const y = new Float32Array(len).fill(0);
    const width = new Float32Array(len).fill(0);
    const height = new Float32Array(len).fill(0);

    const align = new Array(len);
    let anyAlign = false;

    const baseline = new Array(len);
    let anyBaseline = false;

    const zindex = new Int32Array(len).fill(0);
    let anyZindex = false;

    for (let i = 0; i < items.length; i++) {
        let item = items[i];

        if (item.url != null) {
            let url;
            if (item.url.startsWith("data/")) {
                url = "https://vega.github.io/vega-datasets/" + item.url;
            } else {
                url = item.url;
            }
            image[i] = await fetchImage(url, loader);
        }

        if (item.x != null) {
            x[i] = item.x;
        }
        if (item.y != null) {
            y[i] = item.y;
        }

        if (item.width != null) {
            width[i] = item.width;
        } else if (item.x2 != null) {
            width[i] = item.x2 - x[i];
        }

        if (item.height != null) {
            height[i] = item.height;
        } else if (item.y2 != null) {
            height[i] = item.y2 - y[i];
        }

        if (item.align != null) {
            align[i] = item.align;
            anyAlign ||= true;
        }

        if (item.baseline != null) {
            baseline[i] = item.baseline;
            anyBaseline ||= true;
        }

        if (item.zindex != null) {
            zindex[i] = item.zindex;
            anyZindex ||= true;
        }
    }

    imageMark.set_xy(x, y);
    imageMark.set_width(width);
    imageMark.set_height(height);
    imageMark.set_image(image);

    if (anyAlign) {
        const encoded = encodeSimpleArray(align);
        imageMark.set_align(encoded.values, encoded.indices);
    }

    if (anyBaseline) {
        const encoded = encodeSimpleArray(baseline);
        imageMark.set_baseline(encoded.values, encoded.indices);
    }

    if (anyZindex) {
        imageMark.set_zindex(zindex);
    }
    return imageMark;
}

/**
 * @typedef {Object} RgbaImage
 * @property {number} width - The width of the image in pixels.
 * @property {number} height - The height of the image in pixels.
 * @property {Uint8Array} data - The RGBA data of the image.
 */

/**
 * Cache for storing image data promises. The keys are image URLs (strings),
 * and the values are promises that resolve to RgbaImage objects.
 * @type {Map<string, Promise<RgbaImage>>}
 */
const imageCache = new Map();

/**
 * Fetches an image from the specified URL and returns its RGBA data.
 * If the image has been fetched before, the cached result will be used.
 * @param {string} url - The URL of the image to fetch.
 * @param {IResourceLoader} loader
 * @returns {Promise<RgbaImage>} A promise that resolves with the RGBA data of the image.
 */
async function fetchImage(url, loader) {
    // Check if the image data is already cached in the Map
    if (imageCache.has(url)) {
        return imageCache.get(url);
    }

    // Fetch and process the image, then cache the promise in the Map
    const imagePromise = performFetchImage(url, loader);
    imageCache.set(url, imagePromise);
    return imagePromise;
}

/**
 * Fetches and processes the image to extract RGBA data using a given resource loader.
 * @param {string} url - The URL of the image to fetch.
 * @param {IResourceLoader} resourceLoader - The resource loader instance to use for loading the image.
 * @returns {Promise<RgbaImage>} A promise that resolves with the RGBA data of the image.
 */
async function performFetchImage(url, resourceLoader) {
    try {
        const img = await resourceLoader.loadImage(url);
        await resourceLoader.ready();

        return new Promise((resolve, reject) => {
            if (!img.complete || img.naturalWidth === 0) {
                reject(new Error("Failed to load image."));
            }

            const canvas = document.createElement('canvas');
            const ctx = canvas.getContext('2d');
            canvas.width = img.width;
            canvas.height = img.height;
            ctx.drawImage(img, 0, 0);
            const imageData = ctx.getImageData(0, 0, img.width, img.height);
            const data = new Uint8Array(imageData.data.buffer);

            resolve({
                width: img.width,
                height: img.height,
                data: data
            });
        });
    } catch (error) {
        console.error("Error fetching image using resource loader:", error);
        throw new Error("Error in resource loader image fetch");
    }
}