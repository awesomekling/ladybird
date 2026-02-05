// Test that super property writes (PutByIdWithThis) work correctly
// after tier-up. This is a regression test for the IR lifter
// dropping the this_value operand from PutNormalByIdWithThis,
// causing super.prop = value to write on the prototype instead
// of the instance.
function test() {
    class A {
        x = 3;
    }
    class B extends A {
        setX() {
            super.x = 10;
        }
    }
    var b = new B();
    b.setX();
    return b.x;
}
test();
test();
