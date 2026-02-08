function five() {
    "use strict";
    return 5;
}
function caller() {
    return five();
}
caller();
caller();
