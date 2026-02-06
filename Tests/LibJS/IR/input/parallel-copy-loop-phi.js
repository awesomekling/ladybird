// Test SSA destruction for loop-carried phis.
// The loop has a phi for the accumulator that merges the initial value
// with the updated value from the back edge.
function sumTo(n) {
    var sum = 0;
    for (var i = 0; i < n; i++) {
        sum = sum + i;
    }
    return sum;
}
sumTo(5);
