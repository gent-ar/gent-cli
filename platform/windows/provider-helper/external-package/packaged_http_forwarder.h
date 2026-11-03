#pragma once
#include <winsock2.h>
#include <windows.h>
#include <string>
#include <functional>
#include <cstdint>
namespace external_bridge {
// One serial HTTP request using an already admitted persistent duplex channel.
// False retires the exact channel; true permits another request. This function
// never opens a TCP upstream: only the host bridge owns that fixed destination.
bool forward_http(SOCKET client, HANDLE channel, HANDLE owner,
                  const std::string& run_bearer);
bool serve_package_http(HANDLE channel, HANDLE owner, uint16_t port,
    const std::string& run_bearer, unsigned max_connections,
    const std::function<bool()>& listener_ready);
}
