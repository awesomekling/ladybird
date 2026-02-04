// Test constant folding for StrictlyEquals and StrictlyInequals.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = 1.5;
    var b = 1.5;
    var c = a === b;
    var d = 1;
    var e = 1.0;
    var f = d === e;
    var g = null;
    var h = null;
    var i = g === h;
    var j = undefined;
    var k = j === j;
    var l = g !== j;
    return c;
}
go();
