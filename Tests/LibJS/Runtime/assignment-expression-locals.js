describe("simple assignment to local avoids redundant Mov", () => {
    test("assign numeric literal", () => {
        let x = 0;
        x = 42;
        expect(x).toBe(42);
    });

    test("assign string literal", () => {
        let x = "";
        x = "hello";
        expect(x).toBe("hello");
    });

    test("assign from another local", () => {
        let x = 0;
        let y = 99;
        x = y;
        expect(x).toBe(99);
    });

    test("assign binary expression result", () => {
        let x = 0;
        let a = 10;
        let b = 20;
        x = a + b;
        expect(x).toBe(30);
    });

    test("assign complex arithmetic expression", () => {
        let x = 0;
        let a = 5;
        let b = 3;
        x = a * 16 + b;
        expect(x).toBe(83);
    });

    test("assign bitwise expression", () => {
        let x = 0;
        let val = 255;
        x = (val >> 4) & 0xf;
        expect(x).toBe(15);
    });

    test("assign function call result", () => {
        let x = 0;
        function foo() {
            return 77;
        }
        x = foo();
        expect(x).toBe(77);
    });
});

describe("self-referencing assignments must preserve old value", () => {
    test("object literal referencing same variable", () => {
        let obj = { old: "value" };
        obj = { ref: obj };
        expect(obj.ref).toEqual({ old: "value" });
    });

    test("object spread referencing same variable", () => {
        let o = { a: 1, b: 2 };
        o = { ...o, c: 3 };
        expect(o.a).toBe(1);
        expect(o.b).toBe(2);
        expect(o.c).toBe(3);
    });

    test("array literal referencing same variable", () => {
        let arr = [1, 2, 3];
        arr = [arr, 4];
        expect(arr[0]).toEqual([1, 2, 3]);
        expect(arr[1]).toBe(4);
    });

    test("arithmetic using same variable", () => {
        let x = 10;
        x = x + 5;
        expect(x).toBe(15);
    });

    test("assignment where RHS calls function that reads same local", () => {
        let x = 42;
        function getX() {
            return x;
        }
        x = getX() + 1;
        expect(x).toBe(43);
    });

    test("nested object self-reference", () => {
        let config = { debug: true };
        config = { parent: config, debug: false };
        expect(config.parent).toEqual({ debug: true });
        expect(config.debug).toBe(false);
    });

    test("assignment result is the new value", () => {
        let x = 1;
        let y = (x = 5);
        expect(x).toBe(5);
        expect(y).toBe(5);
    });

    test("chained assignment", () => {
        let a = 0;
        let b = 0;
        a = b = 42;
        expect(a).toBe(42);
        expect(b).toBe(42);
    });
});

describe("named function expression assignment", () => {
    test("anonymous function gets name from assignment target", () => {
        let myFunc;
        myFunc = function () {};
        expect(myFunc.name).toBe("myFunc");
    });

    test("function in object property keeps property name", () => {
        let o = { a: function () {} };
        expect(o.a.name).toBe("a");
        let f = o.a;
        expect(f.name).toBe("a");
    });

    test("reassign object with spread preserves function names", () => {
        let o = { a: function () {} };
        let f = o.a;
        o = { ...o, b: f };
        expect(o.a.name).toBe("a");
        expect(o.b.name).toBe("a");
    });
});

describe("var declaration preferred_dst optimization", () => {
    test("var with binary expression initializer", () => {
        var a = 10;
        var b = 20;
        var c = a + b;
        expect(c).toBe(30);
    });

    test("var with bitwise expression initializer", () => {
        var x = 255;
        var y = (x >> 4) & 0xf;
        expect(y).toBe(15);
    });

    test("var with member expression initializer", () => {
        var arr = [10, 20, 30];
        var val = arr[1];
        expect(val).toBe(20);
    });

    test("var with call expression initializer", () => {
        function double(x) {
            return x * 2;
        }
        var result = double(21);
        expect(result).toBe(42);
    });

    test("var self-reference reads old value", () => {
        var x = 10;
        var x = x + 5;
        expect(x).toBe(15);
    });

    test("var in loop body with complex expressions", () => {
        var arr = [1, 2, 3, 4, 5];
        var sum = 0;
        for (var i = 0; i < arr.length; i++) {
            var val = arr[i];
            sum += val;
        }
        expect(sum).toBe(15);
    });

    test("var object literal should not use preferred_dst", () => {
        var x = { a: 1 };
        var x = { old: x };
        expect(x.old).toEqual({ a: 1 });
    });
});
