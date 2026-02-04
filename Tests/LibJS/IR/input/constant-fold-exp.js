// Test constant folding for exponentiation.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = 2;
    var b = 10;
    var c = a ** b;
    return c;
}
go();
