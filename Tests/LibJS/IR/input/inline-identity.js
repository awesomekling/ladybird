function identity(x) {
    "use strict";
    return x;
}
function caller() {
    return identity(42);
}
caller();
caller();
