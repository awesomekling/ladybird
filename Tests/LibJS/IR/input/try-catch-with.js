// Test try/catch with 'with' statement inside try body
try {
    with ({ x: 1 }) {
        throw "error";
    }
} catch (e) {
    e;
}
