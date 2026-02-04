// Test constant folding for Not on non-boolean constants.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var zero = 0;
    var one = 1;
    var a = !zero;
    var b = !one;
    return a !== b;
}
go();
