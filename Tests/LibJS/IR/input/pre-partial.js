// Test PRE with a partially redundant expression.
// typeof is computed on only the true arm (and used), and again after the merge.
// The else arm is explicit to avoid critical edges.
// PRE should insert typeof on the false arm, create a phi, and eliminate the post-merge typeof.
function go(a, b) {
    var x;
    if (b) {
        x = typeof a;
    } else {
        x = "hello";
    }
    var z = typeof a;
    return x + z;
}
go(1, true);
