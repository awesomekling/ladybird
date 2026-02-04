// Test typeof folding when operand type is known.
// Use bitwise OR to force Int32 type, and other patterns for known types.
function go(a) {
    var x = a | 0;
    var b = typeof x === "number";
    var c = typeof x === "object";
    var d = typeof undefined === "undefined";
    var e = typeof true === "boolean";
    return b;
}
go(1);
