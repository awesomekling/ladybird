// Test that tagged template calls round-trip through IR.
function tag(strings, ...values) {
    return strings[0] + values[0] + strings[1];
}
function caller() {
    return tag`hello ${42} world`;
}
caller();
