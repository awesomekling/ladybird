// Test LCSSA with a loop that has multiple exit paths.
// The value 'sum' escapes through both the normal loop exit and the
// early break, requiring LCSSA phis at both exit blocks.
function f(a) {
    var sum = 0;
    for (var i = 0; i < 10; i++) {
        sum = sum + a;
        if (sum > 50) return sum;
    }
    return sum;
}
f(7);
