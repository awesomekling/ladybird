// Test that SCCP resolves phi nodes when only one predecessor edge is
// executable. The outer constant condition means only one branch feeds
// the phi for x, and then that constant propagates into the comparison
// and eliminates the second branch in a single pass.
//
// Without SCCP, the pipeline needs multiple iterations of CopyProp +
// InstCombine + SimplifyCFG to achieve the same result.
function go() {
    var a = 5;
    var b = 10;
    var x;
    if (a) {
        x = b + 1;
    } else {
        x = b - 1;
    }
    // x is always 11 here (through phi resolution of constant-conditioned branch)
    return x;
}
go();
