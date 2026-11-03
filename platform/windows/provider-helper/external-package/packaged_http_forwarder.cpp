#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <windows.h>
#include <array>
#include <algorithm>
#include "packaged_http_forwarder.h"
#include "external_channel_client.h"
#include "contained_http.h"
#include "contained_protocol.h"
#include "stream_response_budget.h"

namespace external_bridge {
namespace {
bool socket_all(SOCKET socket, const char* data, size_t size) {
    while (size) {
        const int written = send(socket, data, static_cast<int>(std::min<size_t>(size, 32768)), 0);
        if (written <= 0) return false;
        data += written; size -= written;
    }
    return true;
}
bool pipe_all(HANDLE pipe, const char* data, size_t size) {
    while (size) {
        DWORD written = 0;
        if (!WriteFile(pipe, data, static_cast<DWORD>(std::min<size_t>(size, 32768)),
            &written, nullptr) || !written) return false;
        data += written; size -= written;
    }
    return true;
}
bool code(HANDLE pipe, uint32_t value) {
    const auto encoded = contained_protocol::encode(value);
    return pipe_all(pipe, encoded.data(), encoded.size());
}
bool data(HANDLE pipe, const char* bytes, size_t size) {
    while (size) {
        const auto count = std::min<size_t>(size, contained_protocol::max_data);
        if (!code(pipe, static_cast<uint32_t>(count)) || !pipe_all(pipe, bytes, count)) return false;
        bytes += count; size -= count;
    }
    return true;
}
bool constant_equal(const std::string& a, const std::string& b) {
    if (a.size() != b.size()) return false;
    unsigned difference = 0;
    for (size_t i = 0; i < a.size(); ++i)
        difference |= static_cast<unsigned char>(a[i]) ^ static_cast<unsigned char>(b[i]);
    return difference == 0;
}
bool authorized(const contained_http::RequestHead& head, const std::string& bearer) {
    if (bearer.empty() || bearer.size() > 256) return false;
    for (const auto& field : head.fields) {
        if (field.first != "authorization") continue;
        const auto colon = field.second.find(':');
        auto value = field.second.substr(colon + 1);
        while (!value.empty() && value.front() == ' ') value.erase(0, 1);
        while (!value.empty() && value.back() == ' ') value.pop_back();
        return constant_equal(value, "Bearer " + bearer);
    }
    return false;
}
bool reject(SOCKET client) {
    constexpr char response[] = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    socket_all(client, response, sizeof(response) - 1);
    return true; // No channel bytes were emitted.
}
bool send_response_bounded(SOCKET client, const char* bytes, size_t size, HANDLE owner,
                           const contained_stream::Budget& budget) {
    while (size) {
        if (budget.expired(GetTickCount64()) || WaitForSingleObject(owner, 0) != WAIT_TIMEOUT) return false;
        const int count = send(client, bytes, static_cast<int>(std::min<size_t>(size, 32768)), 0);
        if (count == SOCKET_ERROR && WSAGetLastError() == WSAEWOULDBLOCK) {
            if (WaitForSingleObject(owner, 10) != WAIT_TIMEOUT) return false;
            continue;
        }
        if (count <= 0) return false;
        bytes += count; size -= count;
    }
    return true;
}
bool receive_response(SOCKET client, HANDLE channel, HANDLE owner, bool cancel,
                      contained_stream::Budget budget = contained_stream::Budget(GetTickCount64())) {
    bool abort_sent = false;
    if (cancel) { if (!code(channel, contained_protocol::abort)) return false; abort_sent = true; }
    std::array<char, contained_protocol::max_data> bytes{};
    size_t total = 0;
    u_long nonblocking = 1;
    if (ioctlsocket(client, FIONBIO, &nonblocking)) return false;
    for (;;) {
        const auto deadline = budget.deadline();
        std::array<unsigned char, 4> header{};
        if (!read_bounded(channel, owner, reinterpret_cast<char*>(header.data()), 4, deadline)) return false;
        const auto count = contained_protocol::decode(header);
        if (count == contained_protocol::end || count == contained_protocol::abort)
            return code(channel, contained_protocol::ack); // First host terminal wins.
        if (!contained_protocol::data(count) || total + count > 128 * 1024 * 1024) return false;
        total += count;
        if (!read_bounded(channel, owner, bytes.data(), count, deadline)) return false;
        if (!budget.progress(GetTickCount64())) return false;
        if (!abort_sent && !send_response_bounded(client, bytes.data(), count, owner, budget)) {
            if (!code(channel, contained_protocol::abort)) return false;
            abort_sent = true;
        }
    }
}
}
bool forward_http(SOCKET client, HANDLE channel, HANDLE owner, const std::string& run_bearer) {
    std::string buffered;
    std::array<char, 32768> bytes{};
    size_t split = std::string::npos;
    while ((split = buffered.find("\r\n\r\n")) == std::string::npos) {
        // Bound allocation before reading; preserve a raw TCP-close probe without
        // emitting any request or retiring the persistent bridge channel.
        const auto room = 16384 - buffered.size();
        if (!room) return reject(client);
        const int count = recv(client, bytes.data(), static_cast<int>(std::min(room, bytes.size())), 0);
        if (count <= 0) return true;
        buffered.append(bytes.data(), static_cast<size_t>(count));
    }
    contained_http::RequestHead head;
    if (!contained_http::parse(buffered.substr(0, split + 4), head)
        || !authorized(head, run_bearer)) return reject(client);
    const auto prefix = buffered.size() - (split + 4);
    if (prefix > head.length) return reject(client);
    if (!data(channel, buffered.data(), buffered.size())) return false;
    size_t remaining = head.length - prefix;
    while (remaining) {
        const int count = recv(client, bytes.data(), static_cast<int>(std::min(remaining, bytes.size())), 0);
        if (count <= 0) return receive_response(client, channel, owner, true);
        if (!data(channel, bytes.data(), static_cast<size_t>(count))) return false;
        remaining -= static_cast<size_t>(count);
    }
    if (!code(channel, contained_protocol::end)) return false;
    return receive_response(client, channel, owner, false);
}
bool serve_package_http(HANDLE channel, HANDLE owner, uint16_t port,
    const std::string& run_bearer, unsigned max_connections,
    const std::function<bool()>& listener_ready) {
    if (!port || !max_connections || max_connections > 32 || run_bearer.empty()) return false;
    WSADATA wsa{};
    if (WSAStartup(MAKEWORD(2, 2), &wsa)) return false;
    const SOCKET listener = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (listener == INVALID_SOCKET) { WSACleanup(); return false; }
    BOOL exclusive = TRUE;
    sockaddr_in address{}; address.sin_family = AF_INET;
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK); address.sin_port = htons(port);
    bool ok = !setsockopt(listener, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
        reinterpret_cast<const char*>(&exclusive), sizeof(exclusive))
        && !bind(listener, reinterpret_cast<const sockaddr*>(&address), sizeof(address))
        && !listen(listener, 4) && listener_ready();
    const auto deadline = GetTickCount64() + 120000;
    unsigned completed = 0;
    while (ok && completed < max_connections && GetTickCount64() < deadline
        && WaitForSingleObject(owner, 0) == WAIT_TIMEOUT) {
        fd_set readable; FD_ZERO(&readable); FD_SET(listener, &readable);
        timeval timeout{0, 100000};
        const int available = select(0, &readable, nullptr, nullptr, &timeout);
        if (available == SOCKET_ERROR) { ok = false; break; }
        if (!available) continue;
        const auto client = accept(listener, nullptr, nullptr);
        if (client == INVALID_SOCKET) { ok = false; break; }
        const DWORD io_timeout = 5000;
        ok = !setsockopt(client, SOL_SOCKET, SO_RCVTIMEO,
            reinterpret_cast<const char*>(&io_timeout), sizeof(io_timeout))
            && !setsockopt(client, SOL_SOCKET, SO_SNDTIMEO,
                reinterpret_cast<const char*>(&io_timeout), sizeof(io_timeout));
        if (ok) ok = forward_http(client, channel, owner, run_bearer);
        shutdown(client, SD_BOTH); closesocket(client); ++completed;
    }
    closesocket(listener); WSACleanup();
    return ok && completed == max_connections;
}
}
