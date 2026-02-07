// Test that formal parameters are always seeded in SSA, even when the
// bytecode writes to the parameter's argument slot before reading it in
// linear block order. Without proper parameter seeding, SSA renaming
// would resolve the parameter to undefined on paths where the write
// doesn't dominate the read.
function foo(a, b, c) {
    if (a) {
        b = { x: 1, y: 2 };
    }
    return b.x;
}
foo(false, { x: 10, y: 20 }, 0);
