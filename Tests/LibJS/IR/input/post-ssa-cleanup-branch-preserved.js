// Test PostSSACleanup: split blocks with Branch predecessors are NOT eliminated.
// An if-else with both arms joining at a phi creates split blocks on critical
// edges. These must be preserved since their predecessors are Branch blocks.
function choose(cond, a, b) {
    var result;
    if (cond) {
        result = a + 1;
    } else {
        result = b + 2;
    }
    return result;
}
choose(true, 10, 20);
