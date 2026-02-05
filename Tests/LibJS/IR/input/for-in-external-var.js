// Test that for-in loops with externally-declared loop variables work
// correctly after tier-up. This is a regression test for two bugs:
// 1. Phi coalescing incorrectly coalesced back-edge operands with phi
//    results, causing register conflicts in loops.
// 2. Block ordering: the entry block must be at index 0 in lowered
//    bytecode, even if optimization passes reorder blocks.
function test() {
    var property;
    for (property in "abc");
    return property;
}
test();
test();
