// Test constant folding for ToInt32, ToNumber, ToBoolean.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = 3;
    var b = a | 0;
    var c = !b;
    return c;
}
go();
