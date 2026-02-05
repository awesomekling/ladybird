// Test iterator tuple extraction (GetIterator -> ExtractValue)
let arr = [1, 2, 3];
let sum = 0;
for (let x of arr) {
    sum = sum + x;
}
sum;
