// Test that Call with expression strings round-trips through IR.
function caller() {
    var obj = {};
    obj.toString = function () {
        return "hello";
    };
    return String(obj);
}
caller();
