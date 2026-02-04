// Test constant folding for Negate and UnaryPlus.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = 5;
    var b = -a;
    var c = +a;
    return b + c;
}
go();
