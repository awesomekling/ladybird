// Test type refinement: nullish guard eliminates redundant checks.
// After branching on x == null (IsNullish), the false branch knows
// x is not null/undefined, so ThrowIfNullish can be removed.
function go(x) {
    if (x == null) {
        return "nullish";
    }
    return x.toString();
}
go("hello");
