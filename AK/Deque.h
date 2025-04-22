/*
 * Copyright (c) 2025, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/AllOf.h>
#include <AK/Function.h>
#include <AK/Iterator.h>

namespace AK {

template<typename T>
class Deque {
public:
    Deque() = default;
    ~Deque()
    {
        clear();
        operator delete[](m_buffer);
    }

    bool is_empty() const { return m_size == 0; }
    size_t size() const { return m_size; }

    void clear()
    {
        for (size_t i = 0; i < m_size; ++i)
            at_unchecked(i).~T();
        m_head = 0;
        m_tail = 0;
        m_size = 0;
    }

    T& first()
    {
        VERIFY(!is_empty());
        return m_buffer[m_head];
    }

    T const& first() const
    {
        VERIFY(!is_empty());
        return m_buffer[m_head];
    }

    T& last()
    {
        VERIFY(!is_empty());
        return m_buffer[(m_tail - 1 + m_capacity) % m_capacity];
    }

    T const& last() const
    {
        VERIFY(!is_empty());
        return m_buffer[(m_tail - 1 + m_capacity) % m_capacity];
    }

    void append(T value)
    {
        ensure_capacity_for_insert();
        new (&m_buffer[m_tail]) T(move(value));
        m_tail = (m_tail + 1) % m_capacity;
        ++m_size;
    }

    void prepend(T value)
    {
        ensure_capacity_for_insert();
        m_head = (m_head - 1 + m_capacity) % m_capacity;
        new (&m_buffer[m_head]) T(move(value));
        ++m_size;
    }

    T take_first()
    {
        VERIFY(!is_empty());
        T result = move(m_buffer[m_head]);
        m_buffer[m_head].~T();
        m_head = (m_head + 1) % m_capacity;
        --m_size;
        return result;
    }

    T take_last()
    {
        VERIFY(!is_empty());
        m_tail = (m_tail - 1 + m_capacity) % m_capacity;
        T result = move(m_buffer[m_tail]);
        m_buffer[m_tail].~T();
        --m_size;
        return result;
    }

    T& operator[](size_t index)
    {
        VERIFY(index < m_size);
        return at_unchecked(index);
    }

    T const& operator[](size_t index) const
    {
        VERIFY(index < m_size);
        return at_unchecked(index);
    }

private:
    T& at_unchecked(size_t index) const
    {
        return m_buffer[(m_head + index) % m_capacity];
    }

    void ensure_capacity_for_insert()
    {
        if (m_size == m_capacity)
            resize(m_capacity == 0 ? 4 : m_capacity * 2);
    }

    void resize(size_t new_capacity)
    {
        VERIFY(new_capacity >= m_size);
        T* new_buffer = static_cast<T*>(operator new[](new_capacity * sizeof(T)));

        for (size_t i = 0; i < m_size; ++i)
            new (&new_buffer[i]) T(move(at_unchecked(i)));

        for (size_t i = 0; i < m_size; ++i)
            at_unchecked(i).~T();

        operator delete[](m_buffer);
        m_buffer = new_buffer;
        m_capacity = new_capacity;
        m_head = 0;
        m_tail = m_size;
    }

    T* m_buffer { nullptr };
    size_t m_capacity { 0 };
    size_t m_head { 0 };
    size_t m_tail { 0 };
    size_t m_size { 0 };
};

}
