// Test ADCE: diamond pattern where the immediate post-dominator is
// neither the true nor false target. Both branches compute dead
// values, so ADCE should eliminate the branch and jump to one target.
// The function call after the merge forces a clean diamond in the IR.
function go(x) {
    var y;
    if (x) {
        y = 1;
    } else {
        y = 2;
    }
    bar();
    return 42;
}
go(true);
