// Test that StrictlyEquals is correctly value-numbered by GVN.
function go(a, b) {
    var x = a === b;
    var y = a === b;
    return x === y;
}
go(1, 2);
