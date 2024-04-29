/**
 * Encode an array of strings as an array of unique strings and a Uint32Array
 * of indices into the array of unique strings.
 *
 * @param {(string|number)[]} originalArray
 * @returns {{indices: Uint32Array, values: string[]}}
 */
export function encodeSimpleArray(originalArray) {
    const uniqueValuesMap = new Map();
    let index = 0;

    // Populate the map with unique strings and their indices
    for (const str of originalArray) {
        if (!uniqueValuesMap.has(str)) {
            uniqueValuesMap.set(str, index++);
        }
    }

    // Generate the array of unique strings.
    // Note, Maps preserve the insertion order of their elements
    const uniqueValuesArray = Array.from(uniqueValuesMap.keys());

    // Build index array
    let indices = new Uint32Array(originalArray.length);
    originalArray.forEach((str, i) => {
        indices[i] = uniqueValuesMap.get(str);
    });

    return {
        values: uniqueValuesArray,
        indices,
    };
}
