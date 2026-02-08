function len(arr) {
    "use strict";
    return arr.length;
}
var arr = [1, 2, 3, 4, 5];
function caller() {
    return len(arr);
}
caller();
caller();
