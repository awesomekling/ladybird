// Test type propagation through IR instructions.
// Uses bitwise ops (which always produce int32) to seed known types,
// then verifies arithmetic on typed values produces number,
// and comparisons produce boolean.
function go(a, b) {
    var c = (a | 0) + (b | 0);
    var d = (a | 0) * (b | 0);
    var e = c < d;
    return e;
}
go(3, 4);
