function foo() {
    var arr = [1, 2, 3, 4, 5];
    var i = 0;
    var results = [];
    while (i < 5) {
        results.push(arr[i++]);
    }
    return results;
}
foo();
