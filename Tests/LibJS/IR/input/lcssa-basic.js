// Test that LCSSA inserts a phi at the loop exit for a value defined
// inside the loop that is used after the loop.
function f(n) {
    var total = 0;
    for (var i = 0; i < n; i++) {
        total = total + i;
    }
    return total;
}
f(10);
