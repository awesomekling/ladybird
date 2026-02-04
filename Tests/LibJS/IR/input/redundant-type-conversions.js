// Test elimination of redundant type conversions.
// (a | 0) produces Int32, so ToInt32 on the result is redundant.
// Similarly, (a + b) on known numerics produces Number, so +x is redundant.
function go(a, b) {
    var x = a | 0;
    var y = x | 0;
    return y;
}
go(3, 4);
