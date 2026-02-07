// Test that RedundantCheckElimination removes ToBoolean when the
// operand is already a boolean. !!x on a comparison result produces
// ToBoolean on a boolean value, which is an identity conversion.
function go(a, b) {
    return !!(a === b);
}
go(1, 2);
