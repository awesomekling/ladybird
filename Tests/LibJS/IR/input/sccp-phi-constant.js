// Test that SCCP propagates constants through phi nodes when only one
// branch is reachable, making the phi effectively a constant.
function go() {
    var x;
    if (true) {
        x = 10;
    } else {
        x = 20;
    }
    return x + 5;
}
go();
