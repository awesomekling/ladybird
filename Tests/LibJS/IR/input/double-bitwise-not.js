// Verify that ~~x is not simplified to x when x is not Int32.
// ~~3.7 = 3, not 3.7. The double BitwiseNot acts as ToInt32.
function go(a) {
    return ~~a;
}
go(3.7);
