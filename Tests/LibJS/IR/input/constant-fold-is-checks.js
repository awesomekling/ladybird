// Test constant folding for IsUndefined and IsNullish.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var a = undefined;
    var b = a === undefined;
    var c = null;
    var d = c ?? 42;
    return b;
}
go();
