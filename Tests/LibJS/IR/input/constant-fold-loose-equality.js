// Test constant folding for LooselyEquals and LooselyInequals.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = null;
    var b = undefined;
    var c = a == b;
    var d = a != 5;
    var e = true == 1;
    return c;
}
go();
