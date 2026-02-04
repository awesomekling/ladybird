// Test that for-in iteration works correctly after tier-up.
// This is a regression test for a bug where GetObjectPropertyIterator
// was incorrectly lowered to GetIterator bytecode, causing "not iterable" errors.
let obj = { a: 1, b: 2, c: 3 };
let keys = [];
for (let key in obj) {
    keys.push(key);
}
keys.sort().join(",");
