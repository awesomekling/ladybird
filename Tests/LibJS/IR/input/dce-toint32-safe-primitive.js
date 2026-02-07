// Test that ToInt32 on a safe primitive with unused result is eliminated by DCE.
function go(x) {
    var b = x === 1;
    b | 0;
    return x;
}
go(1);
