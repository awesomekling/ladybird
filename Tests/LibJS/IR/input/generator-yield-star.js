// Test yield* with multiple Yield instructions sharing the same continuation block
function* inner() {
    yield 1;
    yield 2;
}

function* outer() {
    yield* inner();
}

// Call to trigger tier-up and IR dump (don't call .next() to avoid unrelated runtime crash)
outer();
inner();
