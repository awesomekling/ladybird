function foo() {
    var c = new Uint8Array(10);
    function bar() { return c; }
    c[42] = (c[42] + 4) | 0;
    return c;
}

foo();
