import f from "./named-func-decl-default-export.mjs";

export default function namedDefault() {
    return 42;
}

export const passed = f() === 42 && f.name === "namedDefault";
