// Test constant folding for ToBoolean/Not on constant strings.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var empty = "";
    var nonempty = "hello";
    var a = !empty;
    var b = !nonempty;
    var c = !!empty;
    var d = !!nonempty;
    return a;
}
go();
