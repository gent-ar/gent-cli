#pragma once
#include "host_pipe_bridge_io.h"
#include "host_pipe_bridge_http.h"
#include "contained_protocol.h"
#include "stream_response_budget.h"
#include <array>
#include <atomic>
#include <thread>

namespace host_bridge {
inline bool read_exact(const Io& io, HANDLE pipe, char* bytes, size_t size, ULONGLONG deadline) {
    while (size) {
        DWORD count = 0;
        if (!io.pipe(pipe, bytes, static_cast<DWORD>(size), count, false, deadline)) return false;
        bytes += count; size -= count;
    }
    return true;
}
inline bool read_code(const Io& io, HANDLE pipe, uint32_t& code, ULONGLONG deadline) {
    std::array<unsigned char, 4> bytes{};
    if (!read_exact(io, pipe, reinterpret_cast<char*>(bytes.data()), bytes.size(), deadline)) return false;
    code = contained_protocol::decode(bytes); return true;
}
inline bool write_code(const Io& io, HANDLE pipe, uint32_t code, ULONGLONG deadline) {
    const auto bytes = contained_protocol::encode(code);
    return io.pipe_all(pipe, bytes.data(), bytes.size(), deadline);
}
struct Input {
    const Io& io;
    HANDLE pipe;
    ULONGLONG deadline = 0;
    size_t total = 0;
    bool first = true;
    uint32_t next(std::string& bytes) {
        uint32_t code = 0;
        if (first) {
            std::array<unsigned char, 4> header{};
            if (!read_exact(io, pipe, reinterpret_cast<char*>(header.data()), 1, std::numeric_limits<ULONGLONG>::max())) return contained_protocol::ack;
            deadline = GetTickCount64() + 15000; first = false;
            if (!read_exact(io, pipe, reinterpret_cast<char*>(header.data()) + 1, 3, GetTickCount64() + 15000)) return contained_protocol::ack;
            code = contained_protocol::decode(header);
        } else if (!read_code(io, pipe, code, deadline)) return contained_protocol::ack;
        bytes.clear();
        if (!contained_protocol::data(code)) return code;
        total += code;
        if (total > 16777216 + 16384) return contained_protocol::ack;
        bytes.resize(code);
        if (!read_exact(io, pipe, bytes.data(), bytes.size(), deadline)) return contained_protocol::ack;
        return code;
    }
};
inline bool aborted(HANDLE pipe, const Io& io) {
    const auto deadline = GetTickCount64() + 5000;
    uint32_t code = 0;
    return write_code(io, pipe, contained_protocol::abort, deadline)
        && read_code(io, pipe, code, deadline) && code == contained_protocol::ack;
}
inline bool reject_request(Input& input, uint32_t code) {
    std::string bytes;
    while (contained_protocol::data(code)) code = input.next(bytes);
    if (code != contained_protocol::end && code != contained_protocol::abort) return false;
    return aborted(input.pipe, input.io);
}
enum class Outcome { Pending, Success, Abort, Failed };
inline bool response(HANDLE pipe, const Io& io, SOCKET upstream,
                     contained_stream::Budget budget = contained_stream::Budget(GetTickCount64())) {
    Handle stop(CreateEventW(nullptr, TRUE, FALSE, nullptr));
    Handle done(CreateEventW(nullptr, TRUE, FALSE, nullptr));
    if (!stop.value || !done.value) return false;
    std::atomic<Outcome> outcome{Outcome::Pending};
    std::atomic<bool> acknowledged{false};
    std::thread control([&] {
        const Io reader{io.stopped, io.peer, stop.value};
        bool seen_abort = false;
        for (;;) {
            uint32_t code = 0;
            if (!read_code(reader, pipe, code, std::numeric_limits<ULONGLONG>::max())) break;
            if (code == contained_protocol::ack && outcome.load() != Outcome::Pending) {
                acknowledged.store(true); SetEvent(done.value); return;
            }
            if (code != contained_protocol::abort || seen_abort) break;
            seen_abort = true;
            auto expected = Outcome::Pending;
            if (outcome.compare_exchange_strong(expected, Outcome::Abort)) shutdown(upstream, SD_BOTH);
        }
        if (reader.active()) { outcome.store(Outcome::Failed); shutdown(upstream, SD_BOTH); }
        SetEvent(done.value);
    });
    bool terminal = false;
    try {
        std::array<char, contained_protocol::max_data> bytes{};
        size_t total = 0;
        while (outcome.load() == Outcome::Pending) {
            DWORD count = 0;
            const auto deadline = budget.deadline();
            const bool received = io.socket(upstream, bytes.data(), static_cast<DWORD>(bytes.size()), count, false, deadline);
            if (budget.expired(GetTickCount64()) || !received || !count || total + count > 128 * 1024 * 1024) {
                auto expected = Outcome::Pending;
                outcome.compare_exchange_strong(expected, received && !count && !budget.expired(GetTickCount64()) ? Outcome::Success : Outcome::Abort);
                break;
            }
            if (!budget.progress(GetTickCount64())) {
                auto expected = Outcome::Pending;
                outcome.compare_exchange_strong(expected, Outcome::Abort); break;
            }
            total += count;
            const auto send_deadline = budget.deadline();
            if (!write_code(io, pipe, count, send_deadline) || !io.pipe_all(pipe, bytes.data(), count, send_deadline)) {
                outcome.store(Outcome::Failed); break;
            }
        }
        const auto result = outcome.load();
        if (result == Outcome::Success || result == Outcome::Abort) {
            terminal = write_code(io, pipe, result == Outcome::Success ? contained_protocol::end : contained_protocol::abort, GetTickCount64() + 5000);
            if (terminal) io.wait(done.value, GetTickCount64() + 5000);
        }
    } catch (...) {
        outcome.store(Outcome::Failed);
    }
    SetEvent(stop.value); CancelIoEx(pipe, nullptr); control.join();
    return terminal && acknowledged.load() && outcome.load() != Outcome::Failed;
}
inline bool exchange(HANDLE pipe, const Io& io, uint16_t target, const std::string& broker_token) {
    Input input{io, pipe};
    std::string buffered, bytes;
    uint32_t code = input.next(bytes);
    if (!contained_protocol::data(code)) return reject_request(input, code);
    buffered = bytes;
    size_t split = std::string::npos;
    while ((split = buffered.find("\r\n\r\n")) == std::string::npos) {
        if (buffered.size() >= 16384) return reject_request(input, code);
        code = input.next(bytes);
        if (!contained_protocol::data(code)) return reject_request(input, code);
        buffered += bytes;
    }
    RequestHead head;
    const auto prefix = buffered.substr(split + 4);
    if (!request_head(buffered.substr(0, split + 4), target, broker_token, head) || prefix.size() > head.length) return reject_request(input, code);
    input.deadline = GetTickCount64() + 60000;
    Socket upstream; upstream.value = WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, nullptr, 0, WSA_FLAG_OVERLAPPED);
    if (upstream.value == INVALID_SOCKET || !io.connect(upstream.value, target)) return reject_request(input, code);
    if (!io.socket_all(upstream.value, head.rewritten.data(), head.rewritten.size(), input.deadline)
        || !io.socket_all(upstream.value, prefix.data(), prefix.size(), input.deadline)) return reject_request(input, code);
    size_t left = head.length - prefix.size();
    while (left) {
        code = input.next(bytes);
        if (!contained_protocol::data(code)) return reject_request(input, code);
        if (bytes.size() > left) return reject_request(input, code);
        if (!io.socket_all(upstream.value, bytes.data(), bytes.size(), input.deadline)) return reject_request(input, code);
        left -= bytes.size();
    }
    code = input.next(bytes);
    if (code != contained_protocol::end) return reject_request(input, code);
    shutdown(upstream.value, SD_SEND);
    return response(pipe, io, upstream.value);
}
}
