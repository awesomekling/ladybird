// Test that phi nodes for conditionally-reassigned function arguments
// use the original parameter value, not undefined.
function foo(x) {
    if (x === undefined) x = 42;
    return x;
}
foo(10);
foo(undefined);
