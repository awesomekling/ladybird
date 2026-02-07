// Test constant folding for ToString on primitive constants.
// Use local variables to prevent the bytecode compiler from folding.
function go() {
    var t = true;
    var f = false;
    var n = null;
    var u = undefined;
    var s = "hello";
    var a = `${t}`;
    var b = `${f}`;
    var c = `${n}`;
    var d = `${u}`;
    var e = `${s}`;
    return a;
}
go();
