// Test ADCE: branch where both targets are the same block. This can
// occur after SCCP folds away differences. ADCE should convert the
// branch to a jump without corrupting predecessor lists.
function go(x) {
    var y = x ? 1 : 1;
    return y;
}
go(true);
