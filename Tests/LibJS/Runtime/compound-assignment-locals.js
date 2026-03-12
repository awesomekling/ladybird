describe("compound assignment to local with local RHS (no defensive copy)", () => {
    test("add assign with local", () => {
        let x = 10;
        let y = 5;
        x += y;
        expect(x).toBe(15);
    });

    test("sub assign with local", () => {
        let x = 10;
        let y = 3;
        x -= y;
        expect(x).toBe(7);
    });

    test("mul assign with local", () => {
        let x = 4;
        let y = 7;
        x *= y;
        expect(x).toBe(28);
    });

    test("div assign with local", () => {
        let x = 20;
        let y = 4;
        x /= y;
        expect(x).toBe(5);
    });

    test("mod assign with local", () => {
        let x = 17;
        let y = 5;
        x %= y;
        expect(x).toBe(2);
    });

    test("bitwise AND assign with local", () => {
        let x = 0xff;
        let y = 0x0f;
        x &= y;
        expect(x).toBe(0x0f);
    });

    test("bitwise OR assign with local", () => {
        let x = 0xf0;
        let y = 0x0f;
        x |= y;
        expect(x).toBe(0xff);
    });

    test("bitwise XOR assign with local", () => {
        let x = 0xff;
        let y = 0x0f;
        x ^= y;
        expect(x).toBe(0xf0);
    });

    test("left shift assign with local", () => {
        let x = 1;
        let y = 4;
        x <<= y;
        expect(x).toBe(16);
    });

    test("right shift assign with local", () => {
        let x = 256;
        let y = 4;
        x >>= y;
        expect(x).toBe(16);
    });

    test("unsigned right shift assign with local", () => {
        let x = -1;
        let y = 28;
        x >>>= y;
        expect(x).toBe(15);
    });

    test("self-add (x += x)", () => {
        let x = 21;
        x += x;
        expect(x).toBe(42);
    });

    test("self-multiply (x *= x)", () => {
        let x = 7;
        x *= x;
        expect(x).toBe(49);
    });

    test("double arithmetic compound assign", () => {
        let x = 1.5;
        let y = 2.5;
        x += y;
        expect(x).toBe(4);
    });

    test("string concatenation compound assign", () => {
        let x = "hello";
        let y = " world";
        x += y;
        expect(x).toBe("hello world");
    });
});

describe("compound assignment to local with literal RHS (no defensive copy)", () => {
    test("add assign with number literal", () => {
        let x = 10;
        x += 5;
        expect(x).toBe(15);
    });

    test("sub assign with number literal", () => {
        let x = 10;
        x -= 3;
        expect(x).toBe(7);
    });

    test("mul assign with number literal", () => {
        let x = 6;
        x *= 7;
        expect(x).toBe(42);
    });

    test("add assign with string literal", () => {
        let x = "hello";
        x += " world";
        expect(x).toBe("hello world");
    });

    test("bitwise OR with zero (ToInt32 idiom)", () => {
        let x = 3.7;
        x |= 0;
        expect(x).toBe(3);
    });

    test("compound assign in loop", () => {
        let sum = 0;
        for (let i = 1; i <= 10; i++) {
            sum += i;
        }
        expect(sum).toBe(55);
    });
});

describe("compound assignment with complex RHS (requires defensive copy)", () => {
    test("RHS is a function call that modifies the same variable", () => {
        let x = 10;
        function modify() {
            x = 100;
            return 5;
        }
        x += modify();
        // x was 10 before the RHS was evaluated, so: 10 + 5 = 15
        expect(x).toBe(15);
    });

    test("RHS is a comma expression with side effect on same variable", () => {
        let x = 10;
        x += ((x = 50), 3);
        // x was 10 before RHS, RHS evaluates to 3, so 10 + 3 = 13
        expect(x).toBe(13);
    });

    test("compound assign where RHS is an object expression", () => {
        let x = "prefix";
        x += { toString: () => "-suffix" };
        expect(x).toBe("prefix-suffix");
    });
});

describe("compound assignment with var declarations", () => {
    test("var compound assign in loop (raycasting pattern)", () => {
        var xp = 1.0;
        var yp = 2.0;
        var zp = 3.0;
        var xd = 0.1;
        var yd = 0.2;
        var zd = 0.3;

        for (var i = 0; i < 10; i++) {
            xp += xd;
            yp += yd;
            zp += zd;
        }

        expect(Math.abs(xp - 2.0)).toBeLessThan(1e-10);
        expect(Math.abs(yp - 4.0)).toBeLessThan(1e-10);
        expect(Math.abs(zp - 6.0)).toBeLessThan(1e-10);
    });

    test("compound assign accumulator pattern", () => {
        var result = "";
        var parts = ["a", "b", "c", "d"];
        for (var i = 0; i < parts.length; i++) {
            result += parts[i];
        }
        expect(result).toBe("abcd");
    });
});
