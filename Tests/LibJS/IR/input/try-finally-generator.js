// Test GetException/SetException around yield in finally
function* gen() {
    try {
        throw new Error("oops");
    } finally {
        yield 42;
    }
}
var g = gen();
g.next();
