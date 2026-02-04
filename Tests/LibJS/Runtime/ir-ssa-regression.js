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

test("private field in-check produces correct value", () => {
    // Tests HasPrivateId opcode produces a proper boolean result.
    class Foo {
        #bar = 1;

        static check(obj) {
            return #bar in obj;
        }
    }

    const instance = new Foo();
    const nonInstance = {};

    expect(Foo.check(instance)).toBe(true);
    expect(Foo.check(nonInstance)).toBe(false);

    // Test in conditional context
    class Container {
        #data;

        constructor(value) {
            this.#data = value;
        }

        static getDataOrDefault(obj, defaultValue) {
            if (#data in obj) {
                return obj.#data;
            }
            return defaultValue;
        }
    }

    const c = new Container(42);
    expect(Container.getDataOrDefault(c, 0)).toBe(42);
    expect(Container.getDataOrDefault({}, 0)).toBe(0);
});

test("generator yield preserves values across suspension", () => {
    // Tests PrepareYield correctly passes through values.
    function* counter() {
        let count = 0;
        while (true) {
            count = count + 1;
            yield count;
        }
    }

    const gen = counter();
    expect(gen.next().value).toBe(1);
    expect(gen.next().value).toBe(2);
    expect(gen.next().value).toBe(3);

    // Generator with multiple yields and state
    function* fibonacci() {
        let a = 0;
        let b = 1;
        while (true) {
            yield a;
            const next = a + b;
            a = b;
            b = next;
        }
    }

    const fib = fibonacci();
    expect(fib.next().value).toBe(0);
    expect(fib.next().value).toBe(1);
    expect(fib.next().value).toBe(1);
    expect(fib.next().value).toBe(2);
    expect(fib.next().value).toBe(3);
    expect(fib.next().value).toBe(5);
});

test("EH edges with interleaved definitions", () => {
    // Tests that values defined between multiple throw points are captured correctly.
    let log = [];

    function mayThrow(val, shouldThrow) {
        if (shouldThrow) throw new Error(val);
        return val;
    }

    // Test 1: Value modified between two potential throw points
    // When mayThrow("second", true) throws, x is still "first" because
    // the throw happens before the return/assignment.
    let x = "initial";
    try {
        x = mayThrow("first", false);
        x = mayThrow("second", true);
        x = "unreachable";
    } catch (e) {
        log.push(`caught with x=${x}`);
    }
    expect(log).toEqual(["caught with x=first"]);

    // Test 2: Multiple variables with interleaved modifications
    // When mayThrow(20, true) throws, a=2 and b=10 (from previous assignment)
    log = [];
    let a = 0,
        b = 0;
    try {
        a = 1;
        b = mayThrow(10, false);
        a = 2;
        b = mayThrow(20, true);
        a = 3;
    } catch (e) {
        log.push(`a=${a}, b=${b}`);
    }
    expect(log).toEqual(["a=2, b=10"]);

    // Test 3: Assignment happens before throw
    log = [];
    let z = 0;
    try {
        z = 1;
        throw new Error();
    } catch (e) {
        log.push(`z=${z}`);
    }
    expect(log).toEqual(["z=1"]);
});

test("ToPrimitive side effects preserved in dead code elimination", () => {
    // ToPrimitive side effects must occur even if results are unused.
    let log = [];
    const makeObj = name => ({
        valueOf() {
            log.push(name);
            return 1;
        },
    });

    const a = makeObj("a");
    const b = makeObj("b");
    const c = makeObj("c");

    // Short-circuit: false && ... doesn't evaluate the right side
    if (false && a < 0) {
        // Not reached
    }
    expect(log).toEqual([]);

    // Short-circuit: true || ... doesn't evaluate the right side
    if (true || a < 0) {
        // Short-circuits
    }
    expect(log).toEqual([]);

    // Actual comparison triggers valueOf on both operands
    void (a < b);
    expect(log).toEqual(["a", "b"]);

    // Ternary only evaluates the taken branch for the result
    // The condition `a < 0` calls valueOf on 'a' (returns 1)
    // 1 < 0 is false, so we take the else branch which is just 'c' (no valueOf)
    log = [];
    const result = a < 0 ? b : c;
    expect(log).toEqual(["a"]); // Only 'a' is converted, 'c' is just returned as-is

    // Force valueOf on result
    log = [];
    void (result + 0);
    expect(log).toEqual(["c"]);
});
