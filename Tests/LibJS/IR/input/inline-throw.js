function check(x) {
    "use strict";
    if (x < 0) throw new RangeError("negative");
    return x;
}
function caller() {
    return check(5);
}
caller();
caller();
