// Test that bitwise operations on BigInt produce bigint type, not int32.
function bitwiseNotBigInt() {
    var x = 1n;
    return ~x;
}
bitwiseNotBigInt();
