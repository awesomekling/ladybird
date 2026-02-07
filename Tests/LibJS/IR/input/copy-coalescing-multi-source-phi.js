// Test that CopyCoalescing handles multi-source phi copies correctly.
// The interference graph approach allows coalescing phi destinations
// with different sources on different paths (when safe), which the
// old unique-source guard prevented.
function f(x) {
    var a = x;
    try {
        a = x + 1;
        g();
        a = x + 2;
        h();
    } catch (e) {
        return a + e;
    }
}
f(1);
