// Test self-comparison folding for x === x, x < x, etc.
// Use a parameter to get a non-constant Int32 value.
function go(x) {
    var a = x | 0;
    var b = a === a;
    var c = a !== a;
    var d = a < a;
    var e = a <= a;
    return b;
}
go(42);
