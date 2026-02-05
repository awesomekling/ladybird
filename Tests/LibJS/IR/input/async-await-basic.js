// Test basic async/await IR with Await continuations
async function fetchValue() {
    let x = await Promise.resolve(10);
    let y = await Promise.resolve(20);
    return x + y;
}
fetchValue();
