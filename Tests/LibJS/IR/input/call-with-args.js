// Test Call with multiple arguments round-trips through IR.
function sum(a, b, c) {
    return a + b + c;
}
function caller() {
    return sum(10, 20, 30);
}
caller();
