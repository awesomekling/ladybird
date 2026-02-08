function cube(x) {
    "use strict";
    return x ** 3;
}
function caller() {
    return cube(3);
}
caller();
caller();
