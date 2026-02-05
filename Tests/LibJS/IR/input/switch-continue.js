function foo(x) {
    for (var i = 0; i < 3; i++) {
        switch (i) {
            case 0:
                continue;
            default:
                continue;
        }
    }
    return i;
}
foo(0);
