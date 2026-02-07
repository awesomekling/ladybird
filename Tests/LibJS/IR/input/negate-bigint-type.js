// Test that Negate on BigInt produces bigint type, not number.
// This matters because subsequent operations must not be incorrectly
// treated as pure when operating on a value that is actually a BigInt.
function negateBigInt() {
    var x = 1n;
    var y = -x;
    return +y;
}
try {
    negateBigInt();
} catch (e) {}
