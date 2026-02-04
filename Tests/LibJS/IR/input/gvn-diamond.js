// Test GVN in a diamond CFG.
// typeof computed before the branch should be reused in both arms.
function go(a, b) {
    var x = typeof a;
    if (b) {
        var y = typeof a;
        return x === y;
    } else {
        var z = typeof a;
        return x === z;
    }
}
go(1, true);
