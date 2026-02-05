// Test function with captured variable (produces CreateVariable)
function make_counter() {
    let count = 0;
    return function () {
        return count++;
    };
}
make_counter()();
