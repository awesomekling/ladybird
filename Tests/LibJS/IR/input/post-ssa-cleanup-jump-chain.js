// Test PostSSACleanup: copy blocks in jump chains get eliminated.
// The nested loop structure creates a phi copy block whose predecessor
// ends with Jump after earlier optimization passes merge blocks.
function test(arr) {
    var total = 0;
    for (var i = 0; i < 3; i++) {
        var flag = 1;
        var acc = 0;
        while (flag) {
            if (arr[i] > 0) {
                acc = acc + arr[i];
            }
            flag = 0;
        }
        total = total + acc;
    }
    return total;
}
test([1, 2, 3]);
