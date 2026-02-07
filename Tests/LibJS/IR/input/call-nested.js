// Test nested calls round-trip through IR correctly.
function inner() {
    return 1;
}
function middle() {
    return inner() + 1;
}
function outer() {
    return middle() + 1;
}
outer();
