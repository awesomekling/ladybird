// Verify that x >>> 0 is not incorrectly simplified to x.
// (-1) >>> 0 = 4294967295, which does not fit in Int32.
function go(a) {
    var x = a | 0;
    return x >>> 0;
}
go(-1);
