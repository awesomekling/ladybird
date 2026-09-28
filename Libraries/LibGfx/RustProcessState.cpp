/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Atomic.h>
#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/Mutex.h>
#include <AK/Singleton.h>
#include <AK/Vector.h>
#include <LibGfx/RustProcessState.h>
#include <RustFFI.h>

namespace Gfx {

// NB: Declared in the namespace the definitions below are in, so each has a previous declaration.
extern "C" {
void* ladybird_gfx_decoded_image_frame_retain(void const*, Gfx::FFI::FfiImageFrameSnapshot*);
void ladybird_gfx_decoded_image_frame_release(void*);

void ladybird_gfx_process_note_wanted_pending_face(u64 face_id);
u64 ladybird_gfx_process_set_wanted_face_owner(u64 owner);
void ladybird_gfx_process_take_wanted_pending_faces(u64 owner, void* context, void (*visit)(void*, u64, bool));
void ladybird_gfx_process_requeue_wanted_pending_face(u64 face_id);
u64 ladybird_gfx_process_next_path_identity();
void ladybird_gfx_process_register_image_frame(u64 id, void const* frame);
void ladybird_gfx_process_forget_image_frame(u64 id, void const* frame);
void* ladybird_gfx_process_image_frame_for_id(u64 id, Gfx::FFI::FfiImageFrameSnapshot* out_snapshot);
void ladybird_gfx_process_note_crate_copy(void const* marker);
size_t ladybird_gfx_process_crate_copies_seen();
size_t ladybird_gfx_process_wanted_pending_face_count();
}

namespace {

struct ProcessState {
    Mutex mutex;

    // A face a completed cascade wanted. A want that the document thread could not turn into a
    // load is kept for one more drain: the face may simply not have been reachable yet, and a
    // frozen cascade only ever wants a face once.
    // The owner is the document whose stage wanted the face (0: none said), so that one document's
    // layout end does not take the faces another document's stage, running beside it, noted.
    struct WantedFace {
        u64 face_id { 0 };
        u64 owner { 0 };
        bool has_been_retried { false };
    };
    Vector<WantedFace> wanted_pending_faces;

    Atomic<u64> next_path_identity { 1 };

    // The decoded frames the crate has handles for, so that a frame registered through one copy
    // of it can be found through the other. The registry owns nothing: it points at a frame a
    // handle in one of the copies holds, and hands out a copy of it under this lock, which the
    // registrant takes too before it lets go. Owning one here instead would mean freeing it, and
    // the one thing this registry must never do is decide when a frame dies.
    HashMap<u64, void const*> image_frames;

    HashTable<FlatPtr> crate_copies;
};

Singleton<ProcessState> s_process_state;

// The document whose stage this thread runs, which every want noted here goes to.
thread_local u64 t_wanted_face_owner { 0 };

ProcessState& process_state()
{
    return *s_process_state;
}

}

size_t rust_crate_copies_seen()
{
    auto& state = process_state();
    MutexLocker locker(state.mutex);
    return state.crate_copies.size();
}

extern "C" void ladybird_gfx_process_note_wanted_pending_face(u64 face_id)
{
    auto& state = process_state();
    MutexLocker locker(state.mutex);
    state.wanted_pending_faces.append({ face_id, t_wanted_face_owner, false });
}

extern "C" void ladybird_gfx_process_requeue_wanted_pending_face(u64 face_id)
{
    auto& state = process_state();
    MutexLocker locker(state.mutex);
    state.wanted_pending_faces.append({ face_id, t_wanted_face_owner, true });
}

extern "C" u64 ladybird_gfx_process_set_wanted_face_owner(u64 owner)
{
    return exchange(t_wanted_face_owner, owner);
}

// Takes the wants of `owner`'s stages, and those no stage owned.
extern "C" void ladybird_gfx_process_take_wanted_pending_faces(u64 owner, void* context, void (*visit)(void*, u64, bool))
{
    auto& state = process_state();
    Vector<ProcessState::WantedFace> wanted;
    {
        MutexLocker locker(state.mutex);
        state.wanted_pending_faces.remove_all_matching([&](auto const& face) {
            if (face.owner != owner && face.owner != 0)
                return false;
            wanted.append(face);
            return true;
        });
    }
    for (auto const& face : wanted)
        visit(context, face.face_id, face.has_been_retried);
}

extern "C" u64 ladybird_gfx_process_next_path_identity()
{
    return process_state().next_path_identity.fetch_add(1, AK::MemoryOrder::memory_order_relaxed);
}

extern "C" void ladybird_gfx_process_register_image_frame(u64 id, void const* frame)
{
    auto& state = process_state();
    MutexLocker locker(state.mutex);
    state.image_frames.set(id, frame);
}

extern "C" void ladybird_gfx_process_forget_image_frame(u64 id, void const* frame)
{
    auto& state = process_state();
    MutexLocker locker(state.mutex);
    // Only the handle whose frame is the registered one may take it out. Another copy of the
    // crate may hold a handle for the same id, and it registered a frame of its own.
    if (auto registered = state.image_frames.get(id); registered.has_value() && *registered == frame)
        state.image_frames.remove(id);
}

extern "C" void* ladybird_gfx_process_image_frame_for_id(u64 id, Gfx::FFI::FfiImageFrameSnapshot* out_snapshot)
{
    auto& state = process_state();
    // NB: The retain happens under the lock, so the frame this hands back cannot be released
    //     between the lookup and the copy.
    MutexLocker locker(state.mutex);
    auto registered = state.image_frames.get(id);
    if (!registered.has_value())
        return nullptr;
    return ladybird_gfx_decoded_image_frame_retain(*registered, out_snapshot);
}

extern "C" void ladybird_gfx_process_note_crate_copy(void const* marker)
{
    auto& state = process_state();
    MutexLocker locker(state.mutex);
    state.crate_copies.set(bit_cast<FlatPtr>(marker));
}

// Reachable by name, so that a test can ask the store what it has seen without knowing a C++
// symbol's mangling - and, through each library's own copy of the crate, prove there is one store.
extern "C" size_t ladybird_gfx_process_crate_copies_seen()
{
    return rust_crate_copies_seen();
}

// How many wants are queued. A test uses this to pin that a want the document could not act on is
// offered exactly twice.
extern "C" size_t ladybird_gfx_process_wanted_pending_face_count()
{
    auto& state = process_state();
    MutexLocker locker(state.mutex);
    return state.wanted_pending_faces.size();
}

}
