// Targeted regression tests for IR SSA correctness.
// These tests exercise specific scenarios that have historically been problematic.

test("NaN comparison non-inversion", () => {
    // !(NaN < x) is NOT equivalent to (NaN >= x)
    // The optimizer must not invert comparisons when NaN is involved.
    const x = 5;

    // NaN < anything is false, so !(NaN < x) should be true
    expect(!(NaN < x)).toBe(true);
    // But NaN >= anything is also false
    expect(NaN >= x).toBe(false);
    // These must be different
    expect(!(NaN < x)).not.toBe(NaN >= x);

    // Same for other relational operators
    expect(!(NaN > x)).toBe(true);
    expect(NaN <= x).toBe(false);
    expect(!(NaN > x)).not.toBe(NaN <= x);

    expect(!(NaN <= x)).toBe(true);
    expect(NaN > x).toBe(false);
    expect(!(NaN <= x)).not.toBe(NaN > x);

    expect(!(NaN >= x)).toBe(true);
    expect(NaN < x).toBe(false);
    expect(!(NaN >= x)).not.toBe(NaN < x);
});

test("loop-carried values preserve state correctly", () => {
    // Loop-carried values must not be replaced with undefined.
    // This tests that SSA phi nodes correctly propagate values across loop iterations.
    let sum = 0;
    for (let i = 0; i < 5; i++) {
        sum = sum + i;
    }
    expect(sum).toBe(10); // 0+1+2+3+4 = 10

    // Test with more complex loop-carried state
    let values = [];
    let prev = 0;
    for (let i = 1; i <= 5; i++) {
        values.push(prev);
        prev = i;
    }
    expect(values).toEqual([0, 1, 2, 3, 4]);

    // Nested loops with multiple loop-carried variables
    let result = 0;
    for (let i = 0; i < 3; i++) {
        let inner = i;
        for (let j = 0; j < 3; j++) {
            inner = inner + j;
            result = result + inner;
        }
    }
    expect(result).toBe(21); // Complex accumulation
});

test("exception handler phi correctness", () => {
    // Values at throw points must be correctly captured in exception handlers.
    // This tests that EH edges are properly modeled in SSA.
    let x = 1;
    let capturedX;

    try {
        x = 2;
        throw new Error("test");
    } catch (e) {
        capturedX = x;
    }
    expect(capturedX).toBe(2);

    // More complex: value defined in try, used in catch after throw
    let y = "before";
    let capturedY;
    try {
        y = "after";
        if (true) throw new Error();
        y = "unreachable";
    } catch (e) {
        capturedY = y;
    }
    expect(capturedY).toBe("after");

    // Multiple assignments before throw
    let z = 0;
    let capturedZ;
    try {
        z = 1;
        z = 2;
        z = 3;
        throw new Error();
    } catch (e) {
        capturedZ = z;
    }
    expect(capturedZ).toBe(3);
});

test("algebraic simplification type safety", () => {
    // x - 0 should not be optimized to x for non-numeric types
    // because "5" - 0 = 5 (number), not "5" (string)
    const str = "5";
    const result = str - 0;
    expect(typeof result).toBe("number");
    expect(result).toBe(5);

    // Similarly for other operations that could coerce types
    const obj = { valueOf: () => 42 };
    const objResult = obj - 0;
    expect(typeof objResult).toBe("number");
    expect(objResult).toBe(42);
});

test("side effect preservation in optimization", () => {
    // Operations with side effects must not be incorrectly removed.
    let sideEffectCount = 0;
    const obj = {
        valueOf() {
            sideEffectCount++;
            return 1;
        },
    };

    // Even if the result is unused, side effects must occur
    void (obj + 0);
    expect(sideEffectCount).toBe(1);

    void (obj - 0);
    expect(sideEffectCount).toBe(2);

    // Comparison also calls valueOf
    void (obj < 10);
    expect(sideEffectCount).toBe(3);
});
