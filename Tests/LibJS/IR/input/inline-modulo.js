function isEven(n) {
    "use strict";
    return n % 2 === 0;
}
function caller() {
    return isEven(42);
}
caller();
caller();
