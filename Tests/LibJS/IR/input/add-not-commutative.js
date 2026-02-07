// Test that Add is not treated as commutative by GVN, since
// string concatenation is not commutative: "a" + "b" != "b" + "a".
function go(a, b) {
    var x = a + b;
    var y = b + a;
    return x + y;
}
go("hello", "world");
