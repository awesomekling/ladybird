// Test that multiple super property accesses in the same function use
// distinct cache indices after tier-up. This is a regression test for
// GetByIdWithThis (and similar WithThis ops) not preserving the bytecode
// cache_index during IR lifting, causing all lookups to share cache
// slot 0 and return the wrong property.
function test() {
    class B {
        method() {
            return 1;
        }
        get x() {
            return 2;
        }
    }
    class C extends B {
        doTest() {
            var methodValue = super.method;
            var xValue = super.x;
            return typeof methodValue + " " + xValue;
        }
    }
    return new C().doTest();
}
test();
test();
