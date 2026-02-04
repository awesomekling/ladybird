// Test constant folding for UnaryPlus and Negate with non-numeric types.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = true;
    var b = +a;
    var c = -a;
    var d = null;
    var e = +d;
    var f = undefined;
    var g = +f;
    return b + c;
}
go();
