#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <array>
#include <string>
#include "probe_network.h"
int net(uint16_t port, bool ipv6, const wchar_t* ipv4) {
    WSADATA data{}; if (WSAStartup(MAKEWORD(2, 2), &data)) return -1;
    SOCKET socket = ::socket(ipv6 ? AF_INET6 : AF_INET, SOCK_STREAM, IPPROTO_TCP); if (socket == INVALID_SOCKET) return WSAGetLastError();
    u_long nonblock = 1; ioctlsocket(socket, FIONBIO, &nonblock);
    sockaddr_in address{}; address.sin_family = AF_INET; address.sin_port = htons(port); address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (ipv4 && InetPtonW(AF_INET, ipv4, &address.sin_addr) != 1) { closesocket(socket); WSACleanup(); return -2; }
    sockaddr_in6 address6{}; address6.sin6_family = AF_INET6; address6.sin6_port = htons(port); address6.sin6_addr = in6addr_loopback;
    int result = connect(socket, ipv6 ? reinterpret_cast<sockaddr*>(&address6) : reinterpret_cast<sockaddr*>(&address), ipv6 ? sizeof(address6) : sizeof(address));
    int error = result ? WSAGetLastError() : 0;
    if (error == WSAEWOULDBLOCK) {
        fd_set write_set, error_set; FD_ZERO(&write_set); FD_ZERO(&error_set); FD_SET(socket, &write_set); FD_SET(socket, &error_set);
        timeval timeout{3, 0}; int ready = select(0, nullptr, &write_set, &error_set, &timeout);
        if (ready > 0) { int length = sizeof(error); if (getsockopt(socket, SOL_SOCKET, SO_ERROR, reinterpret_cast<char*>(&error), &length)) error = WSAGetLastError(); }
        else error = ready == 0 ? WSAETIMEDOUT : WSAGetLastError();
    }
    closesocket(socket); WSACleanup(); return error;
}
uint16_t gateway_port() {
    std::wstring origin(1024, L'\0'); DWORD length = GetEnvironmentVariableW(L"ANTHROPIC_BASE_URL", origin.data(), static_cast<DWORD>(origin.size()));
    if (!length || length >= origin.size()) return 0; origin.resize(length); auto colon = origin.find_last_of(L':');
    if (colon == std::wstring::npos) return 0; auto port = std::stoul(origin.substr(colon + 1)); return port && port <= 65535 ? static_cast<uint16_t>(port) : 0;
}
bool gateway_http(uint16_t port) {
    WSADATA data{}; if (WSAStartup(MAKEWORD(2, 2), &data)) return false;
    SOCKET connection = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP); if (connection == INVALID_SOCKET) { WSACleanup(); return false; }
    DWORD timeout = 5000; setsockopt(connection, SOL_SOCKET, SO_RCVTIMEO, reinterpret_cast<char*>(&timeout), sizeof(timeout));
    sockaddr_in address{}; address.sin_family = AF_INET; address.sin_port = htons(port); address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    bool ok = connect(connection, reinterpret_cast<sockaddr*>(&address), sizeof(address)) == 0;
    std::string request = "GET /v1/models HTTP/1.1\r\nHost: 127.0.0.1:" + std::to_string(port) + "\r\nAuthorization: Bearer " + std::string(64, 'b') + "\r\nConnection: close\r\n\r\n";
    if (ok) ok = send(connection, request.data(), static_cast<int>(request.size()), 0) == static_cast<int>(request.size());
    std::string reply; std::array<char, 1024> bytes{}; int n = 0;
    if (ok) while ((n = recv(connection, bytes.data(), static_cast<int>(bytes.size()), 0)) > 0 && reply.size() < 65536) reply.append(bytes.data(), n);
    closesocket(connection); WSACleanup(); return ok && reply.find("HTTP/1.1 200 ") == 0 && reply.size() >= 2 && reply.substr(reply.size() - 2) == "OK";
}
static SOCKET inbound_socket = INVALID_SOCKET;
static std::wstring inbound_ack;
bool begin_inbound_probe(const std::wstring& state_root) {
    WSADATA data{}; if (WSAStartup(MAKEWORD(2, 2), &data)) return false;
    inbound_ack = state_root + L"\\tmp\\listener-checked";
    inbound_socket = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP); sockaddr_in address{}; address.sin_family = AF_INET; address.sin_port = htons(45682); address.sin_addr.s_addr = htonl(INADDR_ANY);
    bool bound = inbound_socket != INVALID_SOCKET && bind(inbound_socket, reinterpret_cast<sockaddr*>(&address), sizeof(address)) == 0 && listen(inbound_socket, 1) == 0;
    DWORD error = bound ? 0 : WSAGetLastError(); std::string status = bound ? "bound\n" : "denied:" + std::to_string(error) + "\n";
    HANDLE file = CreateFileW((state_root + L"\\tmp\\listener-ready").c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, 0, nullptr); DWORD written = 0;
    if (file != INVALID_HANDLE_VALUE) { WriteFile(file, status.data(), static_cast<DWORD>(status.size()), &written, nullptr); CloseHandle(file); }
    return bound;
}
void finish_inbound_probe() {
    if (inbound_ack.empty()) return;
    for (unsigned i = 0; i < 200 && GetFileAttributesW(inbound_ack.c_str()) == INVALID_FILE_ATTRIBUTES; ++i) Sleep(25);
    if (inbound_socket != INVALID_SOCKET) closesocket(inbound_socket); WSACleanup();
}
