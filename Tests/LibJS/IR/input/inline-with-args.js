// Test that the InlineCalls pass handles calls with multiple arguments.
function add(a, b, c) {
    return a + b + c;
}
function caller() {
    return add(1, 2, 3);
}
caller();
