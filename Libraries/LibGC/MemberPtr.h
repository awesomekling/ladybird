/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Format.h>
#include <AK/Traits.h>
#include <AK/Types.h>
#include <LibGC/Forward.h>
#include <LibGC/Ptr.h>

namespace GC {

// Fires a write barrier for a member pointer field at the given address.
// Looks up the owning cell via block address masking and transitions it
// from OldBlack to OldGray if needed.
GC_API void write_barrier_for_member(void const* member_address);

// MemberRef<T> is like GC::Ref<T>, but fires a write barrier on assignment.
// It must only be used for fields inside Cell-derived classes.
template<typename T>
class MemberRef {
public:
    MemberRef() = delete;

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

    template<typename U>
    MemberRef& operator=(MemberRef<U> const& other)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(other.ptr());
        fire_write_barrier();
        return *this;
    }

    template<typename U>
    MemberRef& operator=(Ref<U> const& other)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(other.ptr());
        fire_write_barrier();
        return *this;
    }

    MemberRef& operator=(T& other)
    {
        m_ptr = &other;
        fire_write_barrier();
        return *this;
    }

    template<typename U>
    MemberRef& operator=(U& other)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = &static_cast<T&>(other);
        fire_write_barrier();
        return *this;
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
    ALWAYS_INLINE void fire_write_barrier()
    {
        write_barrier_for_member(this);
    }

    T* m_ptr { nullptr };
};

// MemberPtr<T> is like GC::Ptr<T>, but fires a write barrier on assignment.
// It must only be used for fields inside Cell-derived classes.
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

    template<typename U>
    MemberPtr& operator=(MemberPtr<U> const& other)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(other.ptr());
        fire_write_barrier();
        return *this;
    }

    template<typename U>
    MemberPtr& operator=(Ptr<U> const& other)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(other.ptr());
        fire_write_barrier();
        return *this;
    }

    MemberPtr& operator=(MemberRef<T> const& other)
    {
        m_ptr = other.ptr();
        fire_write_barrier();
        return *this;
    }

    template<typename U>
    MemberPtr& operator=(MemberRef<U> const& other)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(other.ptr());
        fire_write_barrier();
        return *this;
    }

    MemberPtr& operator=(Ref<T> const& other)
    {
        m_ptr = other.ptr();
        fire_write_barrier();
        return *this;
    }

    template<typename U>
    MemberPtr& operator=(Ref<U> const& other)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(other.ptr());
        fire_write_barrier();
        return *this;
    }

    MemberPtr& operator=(T& other)
    {
        m_ptr = &other;
        fire_write_barrier();
        return *this;
    }

    template<typename U>
    MemberPtr& operator=(U& other)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = &static_cast<T&>(other);
        fire_write_barrier();
        return *this;
    }

    MemberPtr& operator=(T* other)
    {
        m_ptr = other;
        fire_write_barrier();
        return *this;
    }

    template<typename U>
    MemberPtr& operator=(U* other)
    requires(IsConvertible<U*, T*>)
    {
        m_ptr = static_cast<T*>(other);
        fire_write_barrier();
        return *this;
    }

    MemberPtr& operator=(nullptr_t)
    {
        m_ptr = nullptr;
        return *this;
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
    ALWAYS_INLINE void fire_write_barrier()
    {
        write_barrier_for_member(this);
    }

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
