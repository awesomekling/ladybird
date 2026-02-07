// Test Call with no arguments round-trips through IR.
function greet() {
    return 42;
}
function caller() {
    return greet();
}
caller();
