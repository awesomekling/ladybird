// Test generator return() in try-finally executes finally and returns value
function* g() {
    try {
        yield 1;
        unreachable();
    } finally {
        cleanup();
    }
}
var iter = g();
iter.next();
iter.return(42);
