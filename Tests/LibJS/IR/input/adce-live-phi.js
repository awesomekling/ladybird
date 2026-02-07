// Test ADCE: a branch selects between two values via a Phi that is
// used by the return. The branch must NOT be eliminated because the
// Phi's value depends on which path was taken.
function go(x) {
    var result;
    if (x > 0) {
        result = 1;
    } else {
        result = 2;
    }
    return result;
}
go(5);
