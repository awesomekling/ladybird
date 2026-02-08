function clamp(x, lo, hi) {
    "use strict";
    if (x < lo) return lo;
    if (x > hi) return hi;
    return x;
}
function caller() {
    "use strict";
    return clamp(5, 0, 10);
}
caller();
caller();
