// Test yield* with .return() - exercises phi moves for yield continuation blocks.
// The resume value from .return() flows through GetCompletionFields and must
// arrive in the correct register after the generator resumes.
function* inner() {
    yield 10;
    yield 20;
}

function* outer() {
    yield* inner();
}

var iter = outer();
iter.next();
var result = iter.return(42);
