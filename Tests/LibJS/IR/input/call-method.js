// Test Call on a method (GetById + Call pattern) round-trips through IR.
function caller() {
    var obj = {
        foo: function () {
            return 42;
        },
    };
    return obj.foo();
}
caller();
