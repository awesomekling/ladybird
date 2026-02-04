// Verify that -(-x) is not simplified to x when x is not Int32.
// -(-0.0) = 0.0 !== -0.0, so the fold is only safe for Int32.
function go(a) {
    return -(-a);
}
go(-0.0);
