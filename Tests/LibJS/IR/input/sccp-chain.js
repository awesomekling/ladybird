// Test that SCCP propagates constants through chains of computations.
function go() {
    var a = 3;
    var b = a + 4;
    var c = b * 2;
    return c;
}
go();
