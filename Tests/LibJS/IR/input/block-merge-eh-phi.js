// Test that BlockMerging doesn't merge blocks that share an exception handler
// with phi nodes. Merging such blocks would create duplicate phi predecessors.
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
