// Test that object literal methods using super property access work
// correctly after tier-up. This is a regression test for the IR lifter
// dropping the home_object operand from NewFunction instructions,
// causing ResolveSuperBase to fail with "env.has_super_binding()".
function test() {
    var base = {
        getNumber() {
            return 10;
        },
    };
    var derived = {
        getNumber() {
            return 20 + super.getNumber();
        },
    };
    Object.setPrototypeOf(derived, base);
    return derived.getNumber();
}
test();
test();
