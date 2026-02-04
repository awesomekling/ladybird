// Test that typeof on a loop-invariant value is hoisted out of the loop.
function go(a) {
    var result;
    for (var i = 0; i < 10; i++) {
        result = typeof a;
    }
    return result;
}
go(42);
