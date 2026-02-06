// Test that Move instructions with the same operand ARE correctly deduplicated by GVN.
function go(a) {
    var x = a;
    var y = a;
    return x === y;
}
go(1);
