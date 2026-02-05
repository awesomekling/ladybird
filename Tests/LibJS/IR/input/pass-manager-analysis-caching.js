// Test that the pass manager correctly caches analyses across passes.
// This function exercises both GVN and LICM, which both need dominators.
// GVN eliminates the redundant typeof inside the loop,
// and LICM hoists the loop-invariant typeof out of the loop.
function go(a) {
    var result;
    for (var i = 0; i < 10; i++) {
        var x = typeof a;
        var y = typeof a;
        result = x === y;
    }
    return result;
}
go(42);
