// Test that RedundantCheckElimination removes ToString when the
// operand is already a string. Template literal on typeof (which
// returns a string) produces a redundant ToString.
function go(x) {
    return `${typeof x}`;
}
go(1);
