// Test that variable bindings (var) are distinguished from lexical bindings (let/const)
function test_var_binding() {
    var x = 1;
    {
        function hoisted() {
            return "foo";
        }
        x = hoisted();
    }
    return x;
}
test_var_binding();
