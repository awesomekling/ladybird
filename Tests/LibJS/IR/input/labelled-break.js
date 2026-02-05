function foo() {
    label: {
        break label;
        expect().fail();
    }
}
foo();
