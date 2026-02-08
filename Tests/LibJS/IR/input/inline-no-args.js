function constant() {
    "use strict";
    return 42;
}
function caller() {
    return constant();
}
caller();
caller();
