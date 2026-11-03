#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include "package_server.h"
#include "packaged_http_forwarder.h"
#include "package_ports.h"
#include <array>
#include <atomic>
#include <mutex>
#include <thread>
namespace external_bridge {
bool serve_package_channels(const std::vector<HANDLE>& channels, HANDLE owner,
    const std::string& bearer, const std::function<bool(uint16_t)>& ready) {
    if (channels.size() != 8 || bearer.size() != 64) return false;
    WSADATA wsa{};
    if (WSAStartup(MAKEWORD(2, 2), &wsa)) return false;
    const auto listener = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (listener == INVALID_SOCKET) { WSACleanup(); return false; }
    BOOL exclusive = TRUE;
    sockaddr_in address{}; address.sin_family = AF_INET;
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    int size = sizeof(address);
    bool bound = !setsockopt(listener, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
        reinterpret_cast<const char*>(&exclusive), sizeof(exclusive));
    if (bound) {
        bound = false;
        for (unsigned port = package_port_first; port <= package_port_last; ++port) {
            address.sin_port = htons(static_cast<uint16_t>(port));
            if (!bind(listener, reinterpret_cast<const sockaddr*>(&address), sizeof(address))) {
                bound = true; break;
            }
            if (WSAGetLastError() != WSAEADDRINUSE && WSAGetLastError() != WSAEACCES) break;
        }
    }
    bound = bound && !listen(listener, 8)
        && !getsockname(listener, reinterpret_cast<sockaddr*>(&address), &size)
        && package_port_allowed(ntohs(address.sin_port));
    if (!bound || !ready(ntohs(address.sin_port))) {
        closesocket(listener); WSACleanup(); return false;
    }
    std::mutex accepting;
    std::vector<std::thread> workers;
    try {
        for (const auto channel : channels) workers.emplace_back([&, channel] {
            while (WaitForSingleObject(owner, 0) == WAIT_TIMEOUT) {
                SOCKET client = INVALID_SOCKET;
                {
                    std::lock_guard<std::mutex> lock(accepting);
                    fd_set set; FD_ZERO(&set); FD_SET(listener, &set); timeval timeout{0, 100000};
                    const auto available = select(0, &set, nullptr, nullptr, &timeout);
                    if (available == SOCKET_ERROR) ExitProcess(125);
                    if (available > 0) client = accept(listener, nullptr, nullptr);
                }
                if (client == INVALID_SOCKET) continue;
                const DWORD timeout = 15000;
                const bool configured = !setsockopt(client, SOL_SOCKET, SO_RCVTIMEO,
                    reinterpret_cast<const char*>(&timeout), sizeof(timeout))
                    && !setsockopt(client, SOL_SOCKET, SO_SNDTIMEO,
                        reinterpret_cast<const char*>(&timeout), sizeof(timeout));
                const bool reusable = configured && forward_http(client, channel, owner, bearer);
                shutdown(client, SD_BOTH); closesocket(client);
                // A broken authenticated channel cannot leave a live, apparently
                // healthy proxy. Exact self retirement closes all owned channels;
                // host observes the retained process and settles its Job.
                if (!reusable && WaitForSingleObject(owner, 0) == WAIT_TIMEOUT) ExitProcess(125);
            }
        });
    } catch (...) { ExitProcess(125); }
    for (auto& worker : workers) worker.join();
    closesocket(listener); WSACleanup(); return true;
}
}
