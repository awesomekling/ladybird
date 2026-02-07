// Test that the InlineCalls pass runs without crashing on simple call patterns.
// NB: Actual inlining requires runtime profile data, so test-js-ir only
// verifies that the pass handles Call instructions correctly without profiles.
function helper(x) {
    return x + 1;
}
function caller() {
    return helper(42);
}
caller();
