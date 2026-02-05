// Test that PutById and PutByValue preserve base_identifier through
// the IR pipeline. This ensures error messages for property writes on
// null/undefined include the variable name.
function putById(o) {
    o.foo = 1;
}
function putByValue(o, k) {
    o[k] = 1;
}
putById({});
putById({});
putByValue({}, "x");
putByValue({}, "x");
