// Test that SCCP removes unreachable blocks.
// The false branch of the constant-true condition should be eliminated
// entirely, along with the side-effectful code inside it.
function go(y) {
    var x = 1;
    if (x) {
        return y + 1;
    }
    // This code is unreachable — SCCP should remove it.
    return y + 2;
}
go(5);
