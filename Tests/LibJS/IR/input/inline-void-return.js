function consume(x) {
    "use strict";
    return void x;
}
function caller() {
    return consume(42);
}
caller();
caller();
