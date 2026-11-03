#pragma once
#include <winsock2.h>
#include <windows.h>
#include <algorithm>
#include <cstdint>
#include <stdexcept>
#include <string>
#include <utility>
#include <limits>

namespace host_bridge {
struct Handle {
    HANDLE value = nullptr;
    explicit Handle(HANDLE v = nullptr) : value(v) {}
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
    ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
};
struct Socket {
    SOCKET value = INVALID_SOCKET;
    ~Socket() { if (value != INVALID_SOCKET) closesocket(value); }
};
inline void require(bool value, const char* error) {
    if (!value) throw std::runtime_error(error);
}
inline void require_native(BOOL value, DWORD error, const char* stage) {
    if (!value) throw std::runtime_error(std::string(stage) + ":" + std::to_string(error));
}
inline bool duplicate_remote(HANDLE process, HANDLE value, HANDLE* copy) {
    __try {
        return DuplicateHandle(process, value, GetCurrentProcess(), copy, 0, FALSE, DUPLICATE_SAME_ACCESS) != 0;
    } __except (GetExceptionCode() == 0xC0000008 ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        SetLastError(ERROR_INVALID_HANDLE); return false;
    }
}
inline bool close_remote(HANDLE process, HANDLE value) {
    HANDLE copy = nullptr;
    __try {
        const auto closed = DuplicateHandle(process, value, GetCurrentProcess(), &copy, 0, FALSE,
            DUPLICATE_CLOSE_SOURCE | DUPLICATE_SAME_ACCESS);
        if (copy) CloseHandle(copy);
        return closed != 0;
    } __except (GetExceptionCode() == 0xC0000008 ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        SetLastError(ERROR_INVALID_HANDLE); return false;
    }
}
inline DWORD remaining(ULONGLONG deadline) {
    const auto now = GetTickCount64();
    return now >= deadline ? 0 : static_cast<DWORD>(std::min<ULONGLONG>(deadline - now, MAXDWORD - 1));
}
struct Io {
    HANDLE stopped;
    HANDLE peer;
    HANDLE canceled = nullptr;
    bool active() const {
        return WaitForSingleObject(stopped, 0) == WAIT_TIMEOUT
            && WaitForSingleObject(peer, 0) == WAIT_TIMEOUT
            && (!canceled || WaitForSingleObject(canceled, 0) == WAIT_TIMEOUT);
    }
    bool wait(HANDLE event, ULONGLONG deadline) const {
        const auto timeout = deadline == std::numeric_limits<ULONGLONG>::max() ? INFINITE : remaining(deadline);
        if (!timeout) return false;
        HANDLE handles[]{stopped, peer, event, canceled};
        return WaitForMultipleObjects(canceled ? 4 : 3, handles, FALSE, timeout) == WAIT_OBJECT_0 + 2;
    }
    bool pipe(HANDLE pipe, void* bytes, DWORD size, DWORD& transferred,
              bool write, ULONGLONG deadline) const {
        if (!active()) return false;
        Handle event(CreateEventW(nullptr, TRUE, FALSE, nullptr));
        if (!event.value) return false;
        OVERLAPPED operation{}; operation.hEvent = event.value;
        const BOOL done = write ? WriteFile(pipe, bytes, size, &transferred, &operation)
                                : ReadFile(pipe, bytes, size, &transferred, &operation);
        if (done) return transferred != 0;
        if (GetLastError() != ERROR_IO_PENDING) return false;
        const bool completed = wait(event.value, deadline);
        if (!completed) CancelIoEx(pipe, &operation);
        const BOOL settled = GetOverlappedResult(pipe, &operation, &transferred, TRUE);
        return completed && settled && transferred != 0;
    }
    bool socket(SOCKET socket, void* bytes, DWORD size, DWORD& transferred,
                bool write, ULONGLONG deadline) const {
        if (!active()) return false;
        Handle event(CreateEventW(nullptr, TRUE, FALSE, nullptr));
        if (!event.value) return false;
        WSAOVERLAPPED operation{}; operation.hEvent = event.value;
        WSABUF buffer{size, static_cast<char*>(bytes)}; DWORD flags = 0;
        const int result = write ? WSASend(socket, &buffer, 1, &transferred, 0, &operation, nullptr)
                                 : WSARecv(socket, &buffer, 1, &transferred, &flags, &operation, nullptr);
        if (!result) return !write || transferred != 0;
        if (WSAGetLastError() != WSA_IO_PENDING) return false;
        const bool completed = wait(event.value, deadline);
        if (!completed) CancelIoEx(reinterpret_cast<HANDLE>(socket), &operation);
        const BOOL settled = WSAGetOverlappedResult(socket, &operation, &transferred, TRUE, &flags);
        return completed && settled && (!write || transferred != 0);
    }
    bool pipe_all(HANDLE pipe, const char* bytes, size_t size, ULONGLONG deadline) const {
        while (size) {
            DWORD n = 0;
            if (!this->pipe(pipe, const_cast<char*>(bytes), static_cast<DWORD>(std::min<size_t>(size, 32768)), n, true, deadline)) return false;
            bytes += n; size -= n;
        }
        return true;
    }
    bool socket_all(SOCKET socket, const char* bytes, size_t size, ULONGLONG deadline) const {
        while (size) {
            DWORD n = 0;
            if (!this->socket(socket, const_cast<char*>(bytes), static_cast<DWORD>(std::min<size_t>(size, 32768)), n, true, deadline)) return false;
            bytes += n; size -= n;
        }
        return true;
    }
    bool connect(SOCKET socket, uint16_t port) const {
        if (!active()) return false;
        Handle event(CreateEventW(nullptr, TRUE, FALSE, nullptr));
        if (!event.value || WSAEventSelect(socket, event.value, FD_CONNECT)) return false;
        sockaddr_in target{}; target.sin_family = AF_INET;
        target.sin_port = htons(port); target.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        const int result = ::connect(socket, reinterpret_cast<sockaddr*>(&target), sizeof(target));
        bool connected = result == 0;
        if (!connected && WSAGetLastError() == WSAEWOULDBLOCK && wait(event.value, GetTickCount64() + 15000)) {
            WSANETWORKEVENTS events{};
            connected = !WSAEnumNetworkEvents(socket, event.value, &events)
                && (events.lNetworkEvents & FD_CONNECT) && !events.iErrorCode[FD_CONNECT_BIT];
        }
        WSAEventSelect(socket, nullptr, 0);
        return connected;
    }
};
}
