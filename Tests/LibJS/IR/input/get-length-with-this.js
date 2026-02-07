// Test that GetLengthWithThis preserves the this_value operand.
// Accessing super.length in a derived class triggers GetLengthWithThis.
class Base {
    get length() {
        return 42;
    }
}

class Derived extends Base {
    getLength() {
        return super.length;
    }
}

function go() {
    var d = new Derived();
    return d.getLength();
}
go();
