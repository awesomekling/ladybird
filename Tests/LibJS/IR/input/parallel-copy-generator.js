// Test that Yield edge phi copies use an intermediate block.
// The resume value appears in the accumulator after resume, so
// ParallelCopy must execute after the generator resumes.
function* gen(start) {
    var acc = start;
    acc = acc + (yield acc);
    acc = acc + (yield acc);
    return acc;
}
gen(10);
