/*
 * Copyright (c) 2024, Tim Flynn <trflynn89@serenityos.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Checked.h>
#include <LibIPC/Decoder.h>
#include <LibIPC/Message.h>

namespace IPC {

using MessageSizeType = u32;

MessageBuffer::MessageBuffer()
{
}

ErrorOr<void> MessageBuffer::extend_data_capacity(size_t capacity)
{
    m_data.ensure_capacity(m_data.size() + capacity);
    return {};
}

ErrorOr<void> MessageBuffer::append_data(u8 const* values, size_t count)
{
    m_data.append(values, count);
    return {};
}

ErrorOr<void> MessageBuffer::append_file_descriptor(int fd)
{
    auto auto_fd = make_ref_counted<AutoCloseFileDescriptor>(fd);
    m_fds.append(move(auto_fd));
    return {};
}

ErrorOr<void> MessageBuffer::extend(MessageBuffer&& buffer)
{
    m_data.extend(move(buffer.m_data));
    m_fds.extend(move(buffer.m_fds));
    return {};
}

ErrorOr<void> MessageBuffer::transfer_message(Transport& transport)
{
    Checked<MessageSizeType> checked_message_size { m_data.size() };
    if (checked_message_size.has_overflow()) {
        return Error::from_string_literal("Message is too large for IPC encoding");
    }

    transport.post_message(m_data, m_fds);
    return {};
}

}
