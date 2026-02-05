// Test that phi coalescing checks for interference with already-coalesced
// values. When a phi result is chain-coalesced with another phi, the
// interference check must consider the entire coalescing class, not just
// the direct phi result.
//
// This pattern comes from CORDIC-style algorithms where two loop variables
// (x, y) are updated simultaneously, with each new value depending on
// the OLD value of the other variable.
function rotate(n) {
    let x = 100;
    let y = 0;
    for (let i = 0; i < n; i++) {
        let newX = x - (y >> i);
        let newY = (x >> i) + y;
        x = newX;
        y = newY;
    }
    return x * y;
}
rotate(3);
