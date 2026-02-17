function outer() {
    var x = 1;
    function inner() {
<<<<<<< HEAD
        return { x: 42 };
=======
        return {x: 42};
>>>>>>> 238321772e (LibJS: Defer identifier registration for object property keys)
    }
    return x;
}
