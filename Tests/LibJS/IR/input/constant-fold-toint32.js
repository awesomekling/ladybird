// Test constant folding for ToInt32 with large doubles.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = 1e20;
    var b = a | 0;
    return b;
}
go();
