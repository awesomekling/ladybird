// Test that exception handler phi nodes correctly capture variable values
// at the point where an exception is thrown, when there are interleaved
// definitions between multiple potential throw points.
function mayThrow(val, shouldThrow) {
    if (shouldThrow) throw new Error(val);
    return val;
}

function test() {
    let a = 0;
    let b = 0;
    try {
        a = 1;
        b = mayThrow(10, false);
        a = 2;
        b = mayThrow(20, true);
        a = 3;
    } catch (e) {
        return [a, b];
    }
}
test();
