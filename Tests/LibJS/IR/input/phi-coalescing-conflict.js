// Test that phi coalescing doesn't incorrectly coalesce operands
// that represent different values from different control flow paths.
// This is a regression test for a bug where both operands of a phi
// would get coalesced with the phi result, causing incorrect behavior.
let cond = true;
let a = 10;
let b = 20;
let x;
if (cond) {
    x = a + 1;
} else {
    x = b + 2;
}
x + 100;
