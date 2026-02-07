// Test that a simple Call instruction round-trips through IR correctly.
function add(a, b) {
    return a + b;
}
function caller() {
    return add(1, 2);
}
caller();
