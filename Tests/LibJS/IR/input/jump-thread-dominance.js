// Test that JumpThreading doesn't break SSA dominance when the thread target
// doesn't directly use bypassed values but downstream blocks do (live-through).
// The inner loop header branches on 'mode' (a phi with constant from outer
// loop). The true-branch checks arr[i] (independent of inner loop phis), but
// its successor uses 'accum' (an inner loop phi that is live-through).
function test(arr) {
    var total = 0;
    for (var i = 0; i < 3; i++) {
        var mode = 1;
        var accum = 0;
        while (mode) {
            if (arr[i] > 0) {
                accum = accum + arr[i];
            }
            mode = 0;
        }
        total = total + accum;
    }
    return total;
}
test([1, 2, 3]);
