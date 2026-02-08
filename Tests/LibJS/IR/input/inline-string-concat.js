function greet(name) {
    "use strict";
    return "Hello, " + name;
}
function caller() {
    return greet("world");
}
caller();
caller();
