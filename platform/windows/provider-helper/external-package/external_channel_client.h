#pragma once
#include <windows.h>
#include <algorithm>
#include <array>
#include <cstdint>
#include <sstream>
#include <string>

namespace external_bridge {
struct ClientBinding {
    DWORD owner_pid;
    uint64_t owner_birth;
    DWORD session;
    std::string nonce; // Public generation correlation; never the bearer.
};
inline uint64_t process_birth(HANDLE process) {
    FILETIME birth{}, exit{}, kernel{}, user{};
    if (!GetProcessTimes(process, &birth, &exit, &kernel, &user)) return 0;
    return static_cast<uint64_t>(birth.dwLowDateTime)
        | (static_cast<uint64_t>(birth.dwHighDateTime) << 32);
}
inline bool checked_pipe(HANDLE pipe, DWORD owner, DWORD session) {
    // Arbitrary stale descriptors must not trigger strict-handle termination.
    __try {
        DWORD flags = 0; ULONG server = 0, server_session = 0;
        return GetHandleInformation(pipe, &flags) && GetFileType(pipe) == FILE_TYPE_PIPE
            && GetNamedPipeServerProcessId(pipe, &server) && server == owner
            && GetNamedPipeServerSessionId(pipe, &server_session)
            && server_session == session;
    } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
}
// Reads one descriptor through an already opened, bounded, ordinary file.
// The package caller must independently derive ClientBinding from activation.
inline HANDLE admit_descriptor(const std::string& descriptor, const ClientBinding& binding,
                               HANDLE owner) {
    if (descriptor.size() > 1024 || descriptor.empty() || descriptor.back() != '\n'
        || binding.nonce.size() != 32 || !binding.owner_pid || !binding.owner_birth
        || !std::all_of(binding.nonce.begin(), binding.nonce.end(), [](unsigned char c) {
            return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
        })) return nullptr;
    std::istringstream input(descriptor);
    std::string marker, nonce, extra;
    uint64_t target_pid = 0, target_birth = 0, owner_pid = 0, owner_birth = 0;
    uint64_t session = 0, remote = 0;
    if (!(input >> marker >> target_pid >> target_birth >> owner_pid >> owner_birth
            >> session >> nonce >> remote) || (input >> extra)
        || marker != "GENT-BRIDGE-DESCRIPTOR/1" || nonce != binding.nonce
        || target_pid != GetCurrentProcessId() || target_birth != process_birth(GetCurrentProcess())
        || owner_pid != binding.owner_pid || owner_birth != binding.owner_birth
        || session != binding.session || !remote || remote > UINTPTR_MAX
        || GetProcessId(owner) != binding.owner_pid || process_birth(owner) != binding.owner_birth
        || WaitForSingleObject(owner, 0) != WAIT_TIMEOUT) return nullptr;
    DWORD current_session = 0;
    if (!ProcessIdToSessionId(GetCurrentProcessId(), &current_session)
        || current_session != binding.session) return nullptr;
    const auto pipe = reinterpret_cast<HANDLE>(static_cast<uintptr_t>(remote));
    if (!checked_pipe(pipe, binding.owner_pid, binding.session)) return nullptr;
    if (!SetHandleInformation(pipe, HANDLE_FLAG_INHERIT, 0)) return nullptr;
    return pipe; // Exact admitted remote handle; caller owns its eventual close.
}
inline bool read_bounded(HANDLE pipe, HANDLE owner, char* output, DWORD size,
                         ULONGLONG deadline) {
    while (size && GetTickCount64() < deadline
        && WaitForSingleObject(owner, 0) == WAIT_TIMEOUT) {
        DWORD available = 0;
        if (!PeekNamedPipe(pipe, nullptr, 0, nullptr, &available, nullptr)) return false;
        if (!available) { Sleep(10); continue; }
        DWORD read = 0;
        if (!ReadFile(pipe, output, std::min(size, available), &read, nullptr) || !read) return false;
        output += read; size -= read;
    }
    return size == 0;
}
// One proof handshake only. No HTTP forwarding and no production token here.
// ACK is 83 bytes, below the verified empty pipe outbound quota. The caller must
// check that quota before calling; synchronous WriteFile has no fake IO deadline.
inline bool consume_challenge(HANDLE pipe, HANDLE owner, const ClientBinding& binding) {
    constexpr size_t hello_prefix = sizeof("GENT-BRIDGE/1\n") - 1;
    std::array<char, hello_prefix + 32 + 1 + 32> hello{};
    struct WipeHello {
        char* data; size_t size;
        ~WipeHello() { SecureZeroMemory(data, size); }
    } wipe{hello.data(), hello.size()};
    if (!read_bounded(pipe, owner, hello.data(), static_cast<DWORD>(hello.size()),
            GetTickCount64() + 10000)
        || std::string(hello.data(), hello_prefix) != "GENT-BRIDGE/1\n"
        || std::string(hello.data() + hello_prefix, 32) != binding.nonce
        || hello[hello_prefix + 32] != '\n') return false;
    std::string ack = "GENT-BRIDGE-ACK/1\n" + binding.nonce + "\n"
        + std::string(hello.data() + hello_prefix + 33, 32);
    DWORD flags = 0, out_size = 0, written = 0;
    if (!GetNamedPipeInfo(pipe, &flags, &out_size, nullptr, nullptr) || out_size < ack.size()) {
        SecureZeroMemory(ack.data(), ack.size()); return false;
    }
    const bool success = WriteFile(pipe, ack.data(), static_cast<DWORD>(ack.size()),
        &written, nullptr) && written == ack.size();
    SecureZeroMemory(hello.data(), hello.size());
    SecureZeroMemory(ack.data(), ack.size());
    return success;
}
// Post-ACK control message on the authenticated channel, before HTTP frames.
// Never persist/log output; the worker needs the same bearer via trusted launch.
inline bool read_run_bearer(HANDLE pipe, HANDLE owner, std::string& output) {
    constexpr size_t prefix = sizeof("GENT-BRIDGE-AUTH/1\n") - 1;
    std::array<char, prefix + 64 + 1> message{};
    struct Wipe {
        char* data; size_t size;
        ~Wipe() { SecureZeroMemory(data, size); }
    } wipe{message.data(), message.size()};
    output.clear();
    if (!read_bounded(pipe, owner, message.data(), static_cast<DWORD>(message.size()),
            GetTickCount64() + 10000)
        || std::string(message.data(), prefix) != "GENT-BRIDGE-AUTH/1\n"
        || message.back() != '\n'
        || !std::all_of(message.begin() + prefix, message.end() - 1, [](unsigned char c) {
            return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
        })) return false;
    output.assign(message.data() + prefix, 64);
    return true;
}
}
