// Test that SCCP eliminates nested dead branches by propagating constants
// through phi nodes. The inner condition depends on a phi that merges
// values from two branches, but since the outer branch has a constant
// condition, only one phi input is reachable.
function go() {
    var x;
    if (true) {
        x = 1;
    } else {
        x = 0;
    }
    // x is always 1 here. SCCP should figure this out through the phi
    // and eliminate the false branch.
    if (x) {
        return 100;
    } else {
        return 200;
    }
}
go();
