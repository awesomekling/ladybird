// Test basic generator IR with Yield continuations
function* count() {
    yield 1;
    yield 2;
    return 3;
}
let g = count();
g.next();
g.next();
g.next();
