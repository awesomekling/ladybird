// Test that const bindings in for-in heads are in TDZ when used in computed properties
function test_for_in_tdz() {
    try {
        for (const x in { [x]: 1 }) {
        }
        return "no error";
    } catch (e) {
        return e instanceof ReferenceError ? "ReferenceError" : "other";
    }
}
test_for_in_tdz();
