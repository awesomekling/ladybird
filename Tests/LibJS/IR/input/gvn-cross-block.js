// Test GVN across basic blocks.
// typeof is pure and should be deduplicated when one block dominates another.
function go(a, b) {
    var x = typeof a;
    if (b) {
        var y = typeof a;
        return y;
    }
    return x;
}
go(1, true);
