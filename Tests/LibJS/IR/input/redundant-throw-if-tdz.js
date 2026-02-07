// Test that RedundantCheckElimination removes dominated ThrowIfTDZ
// on the same SSA value. Using a let binding in for-in produces
// TDZ checks, and dominated occurrences should be eliminated.
let obj = { a: 1, b: 2 };
let r = "";
for (let k in obj) {
    r += k;
    r += k;
}
r;
