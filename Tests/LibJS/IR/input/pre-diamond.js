// Test PRE in a diamond CFG.
// typeof is computed on both arms (and used), and again after the merge.
// PRE should insert a phi merging the arm results and eliminate the post-merge typeof.
function go(a, b) {
    var x;
    if (b) {
        x = typeof a;
    } else {
        x = typeof a;
    }
    var z = typeof a;
    return x + z;
}
go(1, true);
