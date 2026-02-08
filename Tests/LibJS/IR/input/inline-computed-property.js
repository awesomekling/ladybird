function getAt(arr, i) {
    "use strict";
    return arr[i];
}
var a = [10, 20, 30];
function caller() {
    return getAt(a, 1);
}
caller();
caller();
