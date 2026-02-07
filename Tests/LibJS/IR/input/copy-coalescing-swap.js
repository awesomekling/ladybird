// Test that CopyCoalescing preserves swap patterns.
// In a loop with a swap (a, b) = (b, a), the phi copies form a cycle.
// The interference graph must detect that the swapped values interfere
// and NOT coalesce them into the same register.
function swapLoop(n) {
    let a = 1;
    let b = 2;
    for (let i = 0; i < n; i++) {
        let tmp = a;
        a = b;
        b = tmp;
    }
    return a + b * 10;
}
swapLoop(3);
