// Test yield* when the iterator's return method is null.
// GetMethod must coerce null to undefined (not just GetById).
var iterable = {
    next() {
        return { value: 1, done: false };
    },
    get return() {
        return null;
    },
    [Symbol.iterator]() {
        return iterable;
    },
};
function* gen() {
    yield* iterable;
}
var iter = gen();
iter.next();
iter.return(42);
