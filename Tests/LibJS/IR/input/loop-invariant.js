// Test loop invariant code motion
let x = 10;
let sum = 0;
let n = 5;
for (let i = 0; i < n; i++) {
    // x * 2 is loop-invariant and should be hoisted
    sum += x * 2;
}
sum;
