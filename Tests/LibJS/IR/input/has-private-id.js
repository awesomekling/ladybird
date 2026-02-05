// Test that the HasPrivateId bytecode instruction is correctly lifted
// to a dedicated HasPrivateId IR opcode (not mislifted as HasProperty).
class Foo {
    #secret;
    static check(obj) {
        return #secret in obj;
    }
}
Foo.check(new Foo());
