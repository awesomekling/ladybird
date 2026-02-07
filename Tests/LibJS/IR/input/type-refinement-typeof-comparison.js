// Test type refinement: typeof guard makes conflicting typeof fold.
// After branching on typeof x === "string", the true branch knows
// x is a string, so typeof x === "number" should fold to false.
function go(x) {
    if (typeof x === "string") {
        if (typeof x === "number") {
            return "impossible";
        }
        return "string";
    }
    return "other";
}
go("hello");
