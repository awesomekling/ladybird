function foo() {
    var obj = {
        a: 1,
        get x() {
            return 1;
        },
        set x(v) {},
    };
    return obj.a;
}
foo();
