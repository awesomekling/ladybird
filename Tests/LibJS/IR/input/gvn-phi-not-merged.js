// Test that Phi nodes with the same operands in different order are NOT merged by GVN.
function go(a) {
    var x, y;
    if (a) {
        x = 1;
        y = 2;
    } else {
        x = 2;
        y = 1;
    }
    // x and y have Phis with operands {1, 2} but in different order.
    // GVN must not merge them.
    return x - y;
}
go(true);
