#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <windows.h>
#include <bcrypt.h>
#include <sddl.h>
#include <array>
#include <sstream>
#include <stdexcept>
#include <vector>
#include "package_control.h"
#include "package_ports.h"
#include "external_channel_client.h"
#include "host_pipe_bridge_io.h"


namespace external_bridge {
using host_bridge::Handle;
using host_bridge::Io;
static void require(bool ok, const char* stage) {
    if (!ok) throw std::runtime_error(stage);
}
static bool read_exact(const Io& io, HANDLE pipe, char* data, DWORD size,
                       ULONGLONG deadline) {
    while (size) {
        DWORD read = 0;
        if (!io.pipe(pipe, data, size, read, false, deadline) || !read) return false;
        data += read; size -= read;
    }
    return true;
}
uint16_t start_package_control(HANDLE peer, const PackageGrant& grant,
    const std::wstring& path, const std::string& nonce,
    const std::vector<HANDLE>& channels, const std::string& run_bearer) {
    require(channels.size() == 8 && run_bearer.size() == 64, "package-control-binding");
    require(nonce.size() == 32 && std::all_of(nonce.begin(), nonce.end(), [](unsigned char c) {
        return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
    }), "external-proof-nonce");
    require(GetProcessId(peer) == grant.pid && process_birth(peer) == grant.creation
        && WaitForSingleObject(peer, 0) == WAIT_TIMEOUT, "external-proof-peer");
    std::array<unsigned char, 32> challenge{};
    require(BCryptGenRandom(nullptr, challenge.data(), static_cast<ULONG>(challenge.size()),
        BCRYPT_USE_SYSTEM_PREFERRED_RNG) >= 0, "external-proof-random");
    std::wstring suffix;
    for (const auto byte : challenge) {
        suffix += L"0123456789abcdef"[byte >> 4]; suffix += L"0123456789abcdef"[byte & 15];
    }
    const auto name = L"\\\\.\\pipe\\LOCAL\\GentExternalProof-" + suffix;
    Handle token(nullptr);
    require(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token.value) != 0,
        "external-proof-owner-token");
    DWORD size = 0; GetTokenInformation(token.value, TokenUser, nullptr, 0, &size);
    require(size && size < 65536, "external-proof-owner-size");
    std::vector<unsigned char> user(size);
    require(GetTokenInformation(token.value, TokenUser, user.data(), size, &size) != 0,
        "external-proof-owner-user");
    LPWSTR owner_text = nullptr;
    require(ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(user.data())->User.Sid,
        &owner_text) != 0, "external-proof-owner-sid");
    const auto sddl = std::wstring(L"D:P(A;;GA;;;") + owner_text + L")";
    LocalFree(owner_text);
    PSECURITY_DESCRIPTOR raw_security = nullptr;
    require(ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(), SDDL_REVISION_1,
        &raw_security, nullptr) != 0, "external-proof-security");
    SECURITY_ATTRIBUTES security{sizeof(security), raw_security, FALSE};
    Handle server(CreateNamedPipeW(name.c_str(), PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED
        | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE
        | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 4096, 4096, 5000, &security));
    LocalFree(raw_security);
    require(server.value != INVALID_HANDLE_VALUE, "external-proof-server");
    Handle event(CreateEventW(nullptr, TRUE, FALSE, nullptr));
    require(event.value != nullptr, "external-proof-event");
    OVERLAPPED pending{}; pending.hEvent = event.value;
    const auto connected = ConnectNamedPipe(server.value, &pending);
    const auto error = connected ? ERROR_SUCCESS : GetLastError();
    require(connected || error == ERROR_IO_PENDING, "external-proof-connect");
    Handle client(CreateFileW(name.c_str(), FILE_READ_DATA | FILE_WRITE_DATA
        | FILE_READ_ATTRIBUTES | SYNCHRONIZE, 0, nullptr, OPEN_EXISTING,
        SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS, nullptr));
    const bool signaled = connected || (client.value != INVALID_HANDLE_VALUE
        && WaitForSingleObject(event.value, 5000) == WAIT_OBJECT_0);
    if (!signaled) CancelIoEx(server.value, &pending);
    DWORD transferred = 0;
    const bool settled = connected || GetOverlappedResult(server.value, &pending, &transferred, TRUE);
    require(client.value != INVALID_HANDLE_VALUE && signaled && settled,
        "external-proof-preconnect-settle");
    HANDLE remote = nullptr;
    require(DuplicateHandle(GetCurrentProcess(), client.value, peer, &remote,
        0, FALSE, DUPLICATE_SAME_ACCESS) != 0, "external-proof-mint");
    bool target_closed = false, descriptor_visible = false;
    auto reclaim = [&] {
        if (target_closed) return true;
        HANDLE local = nullptr;
        const bool closed = DuplicateHandle(peer, remote, GetCurrentProcess(), &local,
            0, FALSE, DUPLICATE_SAME_ACCESS | DUPLICATE_CLOSE_SOURCE) != 0;
        if (local) CloseHandle(local);
        target_closed = closed || WaitForSingleObject(peer, 0) == WAIT_OBJECT_0;
        return target_closed;
    };
    try {
        HANDLE raw_roundtrip = nullptr;
        require(DuplicateHandle(peer, remote, GetCurrentProcess(), &raw_roundtrip,
            0, FALSE, DUPLICATE_SAME_ACCESS) != 0, "external-proof-roundtrip");
        Handle roundtrip(raw_roundtrip);
        require(CompareObjectHandles(client.value, roundtrip.value)
            && GetFileType(roundtrip.value) == FILE_TYPE_PIPE, "external-proof-object");
        std::ostringstream descriptor;
        descriptor << "GENT-BRIDGE-DESCRIPTOR/2 " << grant.pid << ' ' << grant.creation
            << ' ' << GetCurrentProcessId() << ' ' << process_birth(GetCurrentProcess())
            << ' ' << grant.session << ' ' << nonce << " 9 "
            << reinterpret_cast<uintptr_t>(remote);
        for (const auto channel : channels) descriptor << ' ' << reinterpret_cast<uintptr_t>(channel);
        descriptor << '\n';
        const auto bytes = descriptor.str();
        require(bytes.size() <= 1024, "external-proof-descriptor-size");
        Handle file(CreateFileW(path.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_WRITE_THROUGH, nullptr));
        require(file.value != INVALID_HANDLE_VALUE, "external-proof-descriptor-create");
        DWORD written = 0;
        require(WriteFile(file.value, bytes.data(), static_cast<DWORD>(bytes.size()), &written,
            nullptr) && written == bytes.size() && FlushFileBuffers(file.value),
            "external-proof-descriptor-write");
        // Once visible, the child may close/reuse its descriptor. Never close a
        // raw remote number after this handoff, even on a timeout/error.
        descriptor_visible = true;
        require(CloseHandle(file.value) != 0, "external-proof-descriptor-close");
        file.value = nullptr;
        Handle stopped(CreateEventW(nullptr, TRUE, FALSE, nullptr));
        require(stopped.value != nullptr, "external-proof-stop-event");
        const Io io{stopped.value, peer}; const auto deadline = GetTickCount64() + 10000;
        const auto secret = std::string(reinterpret_cast<const char*>(challenge.data()), challenge.size());
        const auto hello = "GENT-BRIDGE/1\n" + nonce + "\n" + secret;
        require(io.pipe_all(server.value, hello.data(), hello.size(), deadline), "external-proof-hello");
        const auto expected = "GENT-BRIDGE-ACK/1\n" + nonce + "\n" + secret;
        std::string ack(expected.size(), '\0');
        require(read_exact(io, server.value, ack.data(), static_cast<DWORD>(ack.size()), deadline)
            && ack == expected, "external-proof-ack");
        SecureZeroMemory(challenge.data(), challenge.size());
        const auto auth = "GENT-BRIDGE-AUTH/1\n" + run_bearer + "\n";
        require(io.pipe_all(server.value, auth.data(), auth.size(), GetTickCount64() + 10000),
            "package-control-auth");
        std::string ready;
        const auto ready_deadline = GetTickCount64() + 15000;
        for (;;) {
            char byte = 0;
            require(read_exact(io, server.value, &byte, 1, ready_deadline), "package-control-ready");
            if (byte == '\n') break;
            require(ready.size() < 128 && byte >= 32 && byte < 127, "package-control-ready-bound");
            ready += byte;
        }
        std::istringstream fields(ready);
        std::string marker, ready_nonce, extra; unsigned port = 0;
        require(static_cast<bool>(fields >> marker >> ready_nonce >> port) && !(fields >> extra)
            && marker == "GENT-BRIDGE-READY/1" && ready_nonce == nonce && package_port_allowed(port),
            "package-control-ready-binding");
        // Transfer stays with child after successful ACK. Child closes exactly
        // its admitted handle; host must NOT close a potentially reused number.
        target_closed = true;
        return static_cast<uint16_t>(port);
    } catch (...) {
        SecureZeroMemory(challenge.data(), challenge.size());
        // Caller must settle owned child if rollback cannot be proven. Do not
        // interpret an unsuccessful close as absence of a live capability.
        if (descriptor_visible || !reclaim())
            throw std::runtime_error("external-proof-handoff-requires-child-settlement");
        throw;
    }
}
}
