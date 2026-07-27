function int32_binary_specializations(value) {
    return [
        2 + value,
        value + 2,
        value - 2,
        value * 2,
        value ** 2,
        value / 2,
        value ^ 2,
        value & 2,
        value | 2,
        value << 1,
        value << 2,
        value << 3,
        value << 4,
        value << 5,
        value >> 1,
        value >> 2,
        value >> 3,
        value >> 4,
        value >> 5,
        value >>> 2,
        value % 2,
    ];
}

int32_binary_specializations(8);
