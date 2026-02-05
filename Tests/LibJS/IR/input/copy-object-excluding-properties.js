// Test that object rest destructuring works correctly after tier-up.
// This is a regression test for the IR lifter replacing
// CopyObjectExcludingProperties with a simple Move, causing the rest
// object to be the original instead of a copy without excluded keys.
function test() {
    var o = { a: 1, b: 2, c: 3 };
    var { a, ...rest } = o;
    if (a !== 1) return "FAIL: a !== 1";
    if (rest.b !== 2) return "FAIL: rest.b !== 2";
    if (rest.c !== 3) return "FAIL: rest.c !== 3";
    if (rest.a !== undefined) return "FAIL: rest.a should be undefined";
    return "PASS";
}
test();
test();
