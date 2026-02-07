// Test call results used as arguments to another call.
function double(x) {
    return x * 2;
}
function add(a, b) {
    return a + b;
}
function caller() {
    return add(double(3), double(4));
}
caller();
