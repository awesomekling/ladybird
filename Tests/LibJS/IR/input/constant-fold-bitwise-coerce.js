// Test constant folding for bitwise ops with non-Int32 operands.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = 1.7;
    var b = a | 0;
    var c = true;
    var d = ~c;
    return b + d;
}
go();
