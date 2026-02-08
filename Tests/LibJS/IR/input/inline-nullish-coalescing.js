function fallback(a, b) {
    "use strict";
    return a ?? b;
}
function caller() {
    return fallback(null, 99);
}
caller();
caller();
