// Test that Await edge phi copies use an intermediate block.
// Similar to generator test but for async/await.
async function fetchAll(a, b) {
    var result = await a;
    result = result + (await b);
    return result;
}
fetchAll(Promise.resolve(1), Promise.resolve(2));
