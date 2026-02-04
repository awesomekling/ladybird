// Test constant folding for bitwise operations.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = 5;
    var b = 3;
    var c = a | b;
    var d = a & b;
    var e = a ^ b;
    var f = ~a;
    var g = a << b;
    var h = a >> b;
    return c + d + e + f + g + h;
}
go();
