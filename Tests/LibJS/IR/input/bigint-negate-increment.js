// Test that Negate and Increment on BigInt produce bigint type.
function negateAndIncrement() {
    var x = 1n;
    var y = -x;
    var z = ++y;
    return z;
}
negateAndIncrement();
