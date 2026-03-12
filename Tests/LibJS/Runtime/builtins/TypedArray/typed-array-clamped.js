describe("Uint8ClampedArray clamping behavior", () => {
    test("basic store and load", () => {
        const a = new Uint8ClampedArray(4);
        a[0] = 0;
        a[1] = 128;
        a[2] = 255;
        a[3] = 42;
        expect(a[0]).toBe(0);
        expect(a[1]).toBe(128);
        expect(a[2]).toBe(255);
        expect(a[3]).toBe(42);
    });

    test("int32 values clamped to 0 when negative", () => {
        const a = new Uint8ClampedArray(4);
        a[0] = -1;
        a[1] = -128;
        a[2] = -255;
        a[3] = -1000000;
        expect(a[0]).toBe(0);
        expect(a[1]).toBe(0);
        expect(a[2]).toBe(0);
        expect(a[3]).toBe(0);
    });

    test("int32 values clamped to 255 when above", () => {
        const a = new Uint8ClampedArray(4);
        a[0] = 256;
        a[1] = 300;
        a[2] = 1000;
        a[3] = 1000000;
        expect(a[0]).toBe(255);
        expect(a[1]).toBe(255);
        expect(a[2]).toBe(255);
        expect(a[3]).toBe(255);
    });

    test("double values rounded to nearest even", () => {
        const a = new Uint8ClampedArray(8);
        a[0] = 0.5; // round to even -> 0
        a[1] = 1.5; // round to even -> 2
        a[2] = 2.5; // round to even -> 2
        a[3] = 3.5; // round to even -> 4
        a[4] = 0.4; // round down -> 0
        a[5] = 0.6; // round up -> 1
        a[6] = 254.5; // round to even -> 254
        a[7] = 255.4; // round down -> 255
        expect(a[0]).toBe(0);
        expect(a[1]).toBe(2);
        expect(a[2]).toBe(2);
        expect(a[3]).toBe(4);
        expect(a[4]).toBe(0);
        expect(a[5]).toBe(1);
        expect(a[6]).toBe(254);
        expect(a[7]).toBe(255);
    });

    test("double values clamped when negative", () => {
        const a = new Uint8ClampedArray(4);
        a[0] = -0.1;
        a[1] = -1.5;
        a[2] = -100.7;
        a[3] = -Infinity;
        expect(a[0]).toBe(0);
        expect(a[1]).toBe(0);
        expect(a[2]).toBe(0);
        expect(a[3]).toBe(0);
    });

    test("double values clamped when above 255", () => {
        const a = new Uint8ClampedArray(3);
        a[0] = 255.5;
        a[1] = 300.7;
        a[2] = Infinity;
        expect(a[0]).toBe(255);
        expect(a[1]).toBe(255);
        expect(a[2]).toBe(255);
    });

    test("NaN stored as 0", () => {
        const a = new Uint8ClampedArray(1);
        a[0] = 200;
        a[0] = NaN;
        expect(a[0]).toBe(0);
    });

    test("negative zero stored as 0", () => {
        const a = new Uint8ClampedArray(1);
        a[0] = -0;
        expect(a[0]).toBe(0);
    });

    test("boundary values", () => {
        const a = new Uint8ClampedArray(4);
        a[0] = 0;
        a[1] = 255;
        a[2] = -0.0;
        a[3] = 255.0;
        expect(a[0]).toBe(0);
        expect(a[1]).toBe(255);
        expect(a[2]).toBe(0);
        expect(a[3]).toBe(255);
    });

    test("read from Uint8ClampedArray is same as Uint8Array", () => {
        const clamped = new Uint8ClampedArray([0, 127, 255]);
        const unclamped = new Uint8Array([0, 127, 255]);
        expect(clamped[0]).toBe(unclamped[0]);
        expect(clamped[1]).toBe(unclamped[1]);
        expect(clamped[2]).toBe(unclamped[2]);
    });

    test("compound assignment with clamping", () => {
        const a = new Uint8ClampedArray(3);
        a[0] = 200;
        a[0] += 100; // 300 -> clamped to 255
        expect(a[0]).toBe(255);

        a[1] = 10;
        a[1] -= 20; // -10 -> clamped to 0
        expect(a[1]).toBe(0);

        a[2] = 50;
        a[2] *= 10; // 500 -> clamped to 255
        expect(a[2]).toBe(255);
    });

    test("ImageData-like pixel manipulation", () => {
        // Simulate what chambered.js does: write RGBA pixels
        const pixels = new Uint8ClampedArray(4 * 4);
        for (let i = 0; i < 4; i++) {
            const r = (i * 85) | 0;
            const g = (255 - i * 85) | 0;
            const b = 128;
            pixels[i * 4 + 0] = r;
            pixels[i * 4 + 1] = g;
            pixels[i * 4 + 2] = b;
            pixels[i * 4 + 3] = 255;
        }
        expect(pixels[0]).toBe(0);
        expect(pixels[1]).toBe(255);
        expect(pixels[2]).toBe(128);
        expect(pixels[3]).toBe(255);
        expect(pixels[12]).toBe(255);
        expect(pixels[13]).toBe(0);
    });

    test("store double results from arithmetic", () => {
        const a = new Uint8ClampedArray(4);
        // These produce double results from arithmetic
        a[0] = 100 / 3; // 33.333... -> 33
        a[1] = 200 / 3; // 66.666... -> 67
        a[2] = 500 / 3; // 166.666... -> 167
        a[3] = 1000 / 3; // 333.333... -> 255 (clamped)
        expect(a[0]).toBe(33);
        expect(a[1]).toBe(67);
        expect(a[2]).toBe(167);
        expect(a[3]).toBe(255);
    });

    test("loop writing doubles to Uint8ClampedArray", () => {
        const a = new Uint8ClampedArray(10);
        for (let i = 0; i < 10; i++) {
            a[i] = i * 28.3;
        }
        expect(a[0]).toBe(0);
        expect(a[1]).toBe(28);
        expect(a[5]).toBe(142);
        expect(a[9]).toBe(255); // 254.7 -> 255
    });
});

describe("double stores to other typed arrays (ToInt32 truncation)", () => {
    test("double to Int32Array", () => {
        const a = new Int32Array(4);
        a[0] = 3.7;
        a[1] = -3.7;
        a[2] = 2147483648.5; // > INT32_MAX
        a[3] = NaN;
        expect(a[0]).toBe(3);
        expect(a[1]).toBe(-3);
        // ToInt32 wraps: 2147483648 mod 2^32 = -2147483648
        expect(a[2]).toBe(-2147483648);
        expect(a[3]).toBe(0);
    });

    test("double to Uint8Array", () => {
        const a = new Uint8Array(4);
        a[0] = 3.7;
        a[1] = -1.5;
        a[2] = 256.9;
        a[3] = NaN;
        expect(a[0]).toBe(3);
        expect(a[1]).toBe(255); // ToInt32(-1.5) = -1, then & 0xFF = 255
        expect(a[2]).toBe(0); // ToInt32(256.9) = 256, then & 0xFF = 0
        expect(a[3]).toBe(0);
    });

    test("double to Int8Array", () => {
        const a = new Int8Array(3);
        a[0] = 127.9;
        a[1] = 128.5;
        a[2] = -129.5;
        expect(a[0]).toBe(127);
        expect(a[1]).toBe(-128); // 128 as int8 wraps to -128
        expect(a[2]).toBe(127); // -129 as int8 wraps to 127
    });

    test("double to Uint16Array", () => {
        const a = new Uint16Array(3);
        a[0] = 1000.7;
        a[1] = 65536.5;
        a[2] = -1.5;
        expect(a[0]).toBe(1000);
        expect(a[1]).toBe(0); // wraps
        expect(a[2]).toBe(65535); // -1 & 0xFFFF
    });

    test("double to Int16Array", () => {
        const a = new Int16Array(2);
        a[0] = 32767.9;
        a[1] = 32768.5;
        expect(a[0]).toBe(32767);
        expect(a[1]).toBe(-32768); // wraps
    });

    test("double to Uint32Array", () => {
        const a = new Uint32Array(2);
        a[0] = 3.7;
        a[1] = 4294967296.5;
        expect(a[0]).toBe(3);
        expect(a[1]).toBe(0); // wraps
    });

    test("double to Float64Array", () => {
        const a = new Float64Array(3);
        a[0] = 3.14;
        a[1] = -0.0;
        a[2] = NaN;
        expect(a[0]).toBe(3.14);
        expect(Object.is(a[1], -0)).toBeTrue();
        expect(isNaN(a[2])).toBeTrue();
    });

    test("Infinity to integer typed arrays", () => {
        const i32 = new Int32Array(2);
        const u8 = new Uint8Array(2);
        i32[0] = Infinity;
        i32[1] = -Infinity;
        u8[0] = Infinity;
        u8[1] = -Infinity;
        expect(i32[0]).toBe(0);
        expect(i32[1]).toBe(0);
        expect(u8[0]).toBe(0);
        expect(u8[1]).toBe(0);
    });

    test("Infinity to Uint8ClampedArray", () => {
        const a = new Uint8ClampedArray(2);
        a[0] = Infinity;
        a[1] = -Infinity;
        expect(a[0]).toBe(255);
        expect(a[1]).toBe(0);
    });
});
