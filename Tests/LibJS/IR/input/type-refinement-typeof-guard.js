// Test type refinement: typeof guard allows inner typeof to fold.
// After branching on typeof x === "number", the true branch knows
// x is a number, so typeof x should fold to the constant "number".
function go(x) {
    if (typeof x === "number") {
        return typeof x;
    }
    return "other";
}
go(1);
