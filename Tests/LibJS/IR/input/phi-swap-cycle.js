// Test that phi coalescing and lowering handle swap cycles correctly.
// Two phis form a cycle (a_new = b_old, b_new = a_old) on the back
// edge. Without the cycle check, both get coalesced to the same
// register and the swap becomes a no-op.
function cycle(n) {
    var a = 1;
    var b = 2;
    for (var i = 0; i < n; i++) {
        var old_a = a | 0;
        var old_b = b | 0;
        a = old_b;
        b = old_a;
    }
    return a + b;
}
cycle(1);
