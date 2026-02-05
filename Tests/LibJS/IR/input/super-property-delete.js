// Test that delete super.property is correctly lowered through the IR
// pipeline. This is a regression test for the IR lifter dropping the
// this_value operand from DeleteByIdWithThis, causing the lowered bytecode
// to use DeleteById instead, which loses the ReferenceError semantics.
function test() {
    class A {
        foo() {}
    }
    class B extends A {
        bar() {
            delete super.foo;
        }
    }
    var obj = new B();
    try {
        obj.bar();
        return "did not throw";
    } catch (e) {
        return e instanceof ReferenceError ? "ok" : "wrong error: " + e;
    }
}
test();
test();
