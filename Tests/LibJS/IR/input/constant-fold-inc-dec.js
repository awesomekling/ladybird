// Test constant folding for Increment and Decrement.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = 5;
    ++a;
    var b = 3;
    --b;
    return a + b;
}
go();
