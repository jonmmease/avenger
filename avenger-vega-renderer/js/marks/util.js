
export function encodeStringArray(originalArray) {
    const uniqueStringsMap = new Map();
    let index = 0;

    // Populate the map with unique strings and their indices
    for (const str of originalArray) {
        if (!uniqueStringsMap.has(str)) {
            uniqueStringsMap.set(str, index++);
        }
    }

    // Generate the array of unique strings.
    // Note, Maps preserve the insertion order of their elements
    const uniqueStringsArray = Array.from(uniqueStringsMap.keys());

    // Build index array
    let indices = new Uint32Array(originalArray.length);
    originalArray.forEach((str, i) => {
        indices[i] = uniqueStringsMap.get(str);
    });

    return {
        values: uniqueStringsArray,
        indices,
    };
}
