function sumTo(n) {
    "use strict";
    var sum = 0;
    for (var i = 1; i <= n; i++) sum += i;
    return sum;
}
function caller() {
    return sumTo(10);
}
caller();
caller();
