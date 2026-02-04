// Test constant folding for double arithmetic and mixed int32/double.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = 1.5;
    var b = 2.5;
    var c = a + b;
    var d = a * b;
    var e = d / c;
    return c + e;
}
go();
