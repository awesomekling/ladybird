// Test async generator yield* IR with SetCompletionType, AsyncIteratorClose, NewTypeError
async function* gen() {
    yield* [1, 2];
}
gen();
