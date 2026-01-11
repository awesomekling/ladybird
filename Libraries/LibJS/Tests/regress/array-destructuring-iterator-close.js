test("array destructuring closes iterator when initializer throws", () => {
    let return_called = 0;

    const iterable = {
        [Symbol.iterator]() {
            return {
                next() {
                    return { value: undefined, done: false };
                },
                return() {
                    return_called++;
                    return { done: true };
                },
            };
        },
    };

    const throws = () => {
        throw new Error("boom");
    };

    expect(() => {
        let [value = throws()] = iterable;
    }).toThrowWithMessage(Error, "boom");

    expect(return_called).toBe(1);
});
