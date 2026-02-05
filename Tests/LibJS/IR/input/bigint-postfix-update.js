function foo() {
    var x = 1n;
    var old = x++;
    return old;
}
foo();
