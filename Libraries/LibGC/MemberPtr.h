/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Format.h>
#include <AK/Traits.h>
#include <AK/Types.h>
#include <LibGC/Cell.h>
#include <LibGC/Forward.h>
#include <LibGC/Ptr.h>

namespace GC {

// MemberRef<T> is like GC::Ref<T>, but requires an explicit set() call with
// the owning Cell to fire a write barrier. Used for fields inside Cell-derived
// classes. Construction does not fire a barrier (the cell is being initialized).
template<typename T>
class MemberRef {
public:
    MemberRef() = delete;

    MemberRef(MemberRef const&) = default;
    MemberRef(MemberRef&&) = default;
    MemberRef& operator=(MemberRef const&) = delete;
    MemberRef& operator=(MemberRef&&) = delete;

    MemberRef(T& ptr)
        : m_ptr(&ptr)
    {
    }

    template<typename U>
    MemberRef(U& ptr)
    requires(IsConvertible<U*, T*>)
        : m_ptr(&static_cast<T&>(ptr))
    {
    }

    template<typename U>
    MemberRef(MemberRef<U> const& other)
    requires(IsConvertible<U*, T*>)
        : m_ptr(other.ptr())
    {
    }

    template<typename U>
    MemberRef(Ref<U> const& other)
    requires(IsConvertible<U*, T*>)
        : m_ptr(other.ptr())
    {
    }

    ALWAYS_INLINE void set(Cell& owner, T& value)
    {
        m_ptr = &value;
        owner.write_barrier();
    }

    ALWAYS_INLINE void set(Cell& owner, T* value)
    {
        ASSERT(value);
        m_ptr = value;
        owner.write_barrier();
    }

    template<typename U>
    ALWAYS_INLINE void set(Cell& owner, Ref<U> const& value)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(value.ptr());
        owner.write_barrier();
    }

    template<typename U>
    ALWAYS_INLINE void set(Cell& owner, MemberRef<U> const& value)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(value.ptr());
        owner.write_barrier();
    }

    RETURNS_NONNULL T* operator->() const { return m_ptr; }

    [[nodiscard]] T& operator*() const { return *m_ptr; }

    RETURNS_NONNULL T* ptr() const { return m_ptr; }

    RETURNS_NONNULL operator T*() const { return m_ptr; }

    operator T&() const { return *m_ptr; }

    operator Ref<T>() const { return *m_ptr; }

    operator bool() const = delete;
    bool operator!() const = delete;

private:
    T* m_ptr { nullptr };
};

// MemberPtr<T> is like GC::Ptr<T>, but requires an explicit set() call with
// the owning Cell to fire a write barrier. Used for fields inside Cell-derived
// classes. Construction and nullptr assignment do not fire barriers.
template<typename T>
class MemberPtr {
public:
    constexpr MemberPtr() = default;

    MemberPtr(T& ptr)
        : m_ptr(&ptr)
    {
    }

    MemberPtr(T* ptr)
        : m_ptr(ptr)
    {
    }

    template<typename U>
    MemberPtr(MemberPtr<U> const& other)
    requires(IsConvertible<U*, T*>)
        : m_ptr(other.ptr())
    {
    }

    template<typename U>
    MemberPtr(Ptr<U> const& other)
    requires(IsConvertible<U*, T*>)
        : m_ptr(other.ptr())
    {
    }

    MemberPtr(MemberRef<T> const& other)
        : m_ptr(other.ptr())
    {
    }

    template<typename U>
    MemberPtr(MemberRef<U> const& other)
    requires(IsConvertible<U*, T*>)
        : m_ptr(other.ptr())
    {
    }

    MemberPtr(Ref<T> const& other)
        : m_ptr(other.ptr())
    {
    }

    template<typename U>
    MemberPtr(Ref<U> const& other)
    requires(IsConvertible<U*, T*>)
        : m_ptr(other.ptr())
    {
    }

    MemberPtr(nullptr_t)
        : m_ptr(nullptr)
    {
    }

    // Allow copy/move construction (needed for reading the field value).
    MemberPtr(MemberPtr const&) = default;
    MemberPtr(MemberPtr&&) = default;

    // Prevent implicit copy/move assignment, which would bypass the write barrier.
    MemberPtr& operator=(MemberPtr const&) = delete;
    MemberPtr& operator=(MemberPtr&&) = delete;

    // Clearing to nullptr does not need a barrier.
    MemberPtr& operator=(nullptr_t)
    {
        m_ptr = nullptr;
        return *this;
    }

    // All other assignments must go through set() to fire a write barrier.
    ALWAYS_INLINE void set(Cell& owner, T* value)
    {
        m_ptr = value;
        owner.write_barrier();
    }

    ALWAYS_INLINE void set(Cell& owner, T& value)
    {
        m_ptr = &value;
        owner.write_barrier();
    }

    template<typename U>
    ALWAYS_INLINE void set(Cell& owner, Ptr<U> const& value)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(value.ptr());
        owner.write_barrier();
    }

    template<typename U>
    ALWAYS_INLINE void set(Cell& owner, Ref<U> const& value)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(value.ptr());
        owner.write_barrier();
    }

    template<typename U>
    ALWAYS_INLINE void set(Cell& owner, MemberPtr<U> const& value)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(value.ptr());
        owner.write_barrier();
    }

    template<typename U>
    ALWAYS_INLINE void set(Cell& owner, MemberRef<U> const& value)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(value.ptr());
        owner.write_barrier();
    }

    T* operator->() const
    {
        ASSERT(m_ptr);
        return m_ptr;
    }

    [[nodiscard]] T& operator*() const
    {
        ASSERT(m_ptr);
        return *m_ptr;
    }

    T* ptr() const { return m_ptr; }

    explicit operator bool() const { return !!m_ptr; }
    bool operator!() const { return !m_ptr; }

    operator T*() const { return m_ptr; }

    operator Ptr<T>() const { return m_ptr; }

    Ref<T> as_nonnull() const
    {
        VERIFY(m_ptr);
        return *m_ptr;
    }

private:
    T* m_ptr { nullptr };
};

// Comparison operators
template<typename T, typename U>
inline bool operator==(MemberPtr<T> const& a, MemberPtr<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(MemberPtr<T> const& a, Ptr<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(Ptr<T> const& a, MemberPtr<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(MemberPtr<T> const& a, MemberRef<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(MemberRef<T> const& a, MemberPtr<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(MemberRef<T> const& a, MemberRef<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(MemberPtr<T> const& a, Ref<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(Ref<T> const& a, MemberPtr<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(MemberRef<T> const& a, Ref<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(Ref<T> const& a, MemberRef<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(MemberRef<T> const& a, Ptr<U> const& b) { return a.ptr() == b.ptr(); }

template<typename T, typename U>
inline bool operator==(Ptr<T> const& a, MemberRef<U> const& b) { return a.ptr() == b.ptr(); }

}

namespace AK {

template<typename T>
struct Traits<GC::MemberPtr<T>> : public DefaultTraits<GC::MemberPtr<T>> {
    static unsigned hash(GC::MemberPtr<T> const& value)
    {
        return Traits<T*>::hash(value.ptr());
    }
    static constexpr bool may_have_slow_equality_check() { return false; }
};

template<typename T>
struct Traits<GC::MemberRef<T>> : public DefaultTraits<GC::MemberRef<T>> {
    static unsigned hash(GC::MemberRef<T> const& value)
    {
        return Traits<T*>::hash(value.ptr());
    }
};

template<typename T>
struct Formatter<GC::MemberPtr<T>> : Formatter<T const*> {
    ErrorOr<void> format(FormatBuilder& builder, GC::MemberPtr<T> const& value)
    {
        return Formatter<T const*>::format(builder, value.ptr());
    }
};

template<Formattable T>
struct Formatter<GC::MemberRef<T>> : Formatter<T> {
    ErrorOr<void> format(FormatBuilder& builder, GC::MemberRef<T> const& value)
    {
        return Formatter<T>::format(builder, *value);
    }
};

template<typename T>
requires(!HasFormatter<T>)
struct Formatter<GC::MemberRef<T>> : Formatter<T const*> {
    ErrorOr<void> format(FormatBuilder& builder, GC::MemberRef<T> const& value)
    {
        return Formatter<T const*>::format(builder, value.ptr());
    }
};

}
