#include <winsock2.h>
#include <ws2tcpip.h>
#include "gateway_proxy.h"
#include <algorithm>
#include <array>
#include <atomic>
#include <string>
#include <thread>

static bool send_all(SOCKET socket, const char* data, size_t length) {
    while (length) { int n = send(socket, data, static_cast<int>(std::min<size_t>(length, 32768)), 0); if (n <= 0) return false; data += n; length -= n; } return true;
}
static void relay_socket(SOCKET input, SOCKET output) {
    std::array<char, 32768> data{}; int n = 0;
    while ((n = recv(input, data.data(), static_cast<int>(data.size()), 0)) > 0) if (!send_all(output, data.data(), n)) break;
    shutdown(output, SD_SEND);
}
static bool rewrite_head(SOCKET client, uint16_t target_port, const std::string& broker_token, std::string& output) {
    std::string head; head.reserve(4096);
    while (head.size() < 32768 && (head.size() < 4 || head.compare(head.size() - 4, 4, "\r\n\r\n"))) {
        char byte = 0; if (recv(client, &byte, 1, 0) != 1 || byte == '\0') return false; head += byte;
    }
    if (head.size() >= 32768) return false;
    size_t first = head.find("\r\n"); auto request = head.substr(0, first); auto space = request.find(' ');
    if (space == std::string::npos || request.substr(0, space) == "CONNECT" || space + 1 >= request.size() || request[space + 1] != '/' || request.find(" HTTP/1.1") == std::string::npos) return false;
    output = request + "\r\n"; bool host = false, length = false, encoding = false;
    for (size_t start = first + 2; start < head.size() - 2;) {
        size_t end = head.find("\r\n", start); if (end == std::string::npos || head[start] == ' ' || head[start] == '\t') return false;
        auto line = head.substr(start, end - start); auto colon = line.find(':'); if (colon == std::string::npos) return false;
        auto name = line.substr(0, colon); for (auto& c : name) if (c >= 'A' && c <= 'Z') c = static_cast<char>(c + ('a' - 'A'));
        if (name == "host") { if (host) return false; host = true; }
        else if (name == "connection" || name == "proxy-connection") {}
        else if (!broker_token.empty() && (name == "authorization" || name == "x-api-key")) {}
        else if (name == "upgrade") return false;
        else { if (name == "content-length") { if (length || encoding) return false; length = true; }
            if (name == "transfer-encoding") { if (encoding || length) return false; encoding = true; }
            output += line + "\r\n"; }
        start = end + 2;
    }
    if (!host) return false;
    if (!broker_token.empty()) output += "Authorization: Bearer " + broker_token + "\r\n";
    output += "Host: 127.0.0.1:" + std::to_string(target_port) + "\r\nConnection: close\r\n\r\n"; return true;
}
int serve_gateway(uint16_t host_port, const wchar_t* status_pipe, const std::string& broker_token) {
    if (!broker_token.empty() && (broker_token.size() != 64 || !std::all_of(broker_token.begin(), broker_token.end(), [](char c) { return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'); }))) return 126;
    WSADATA data{}; if (WSAStartup(MAKEWORD(2, 2), &data)) return 121;
    SOCKET listener = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP); if (listener == INVALID_SOCKET) return 122;
    BOOL exclusive = TRUE; setsockopt(listener, SOL_SOCKET, SO_EXCLUSIVEADDRUSE, reinterpret_cast<char*>(&exclusive), sizeof(exclusive));
    sockaddr_in local{}; local.sin_family = AF_INET; local.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (bind(listener, reinterpret_cast<sockaddr*>(&local), sizeof(local)) || listen(listener, 8)) return 123;
    int address_size = sizeof(local); if (getsockname(listener, reinterpret_cast<sockaddr*>(&local), &address_size)) return 124;
    HANDLE status = CreateFileW(status_pipe, GENERIC_WRITE, 0, nullptr, OPEN_EXISTING, 0, nullptr); if (status == INVALID_HANDLE_VALUE) return 125;
    std::string ready = std::to_string(ntohs(local.sin_port)) + "\n"; DWORD written = 0;
    if (!WriteFile(status, ready.data(), static_cast<DWORD>(ready.size()), &written, nullptr) || written != ready.size()) { CloseHandle(status); return 126; } CloseHandle(status);
    auto active = std::make_shared<std::atomic<unsigned>>(0);
    for (;;) {
        SOCKET client = accept(listener, nullptr, nullptr); if (client == INVALID_SOCKET) break;
        if (active->fetch_add(1) >= 8) { active->fetch_sub(1); closesocket(client); continue; }
        std::thread([client, host_port, active, broker_token] {
            DWORD timeout = 15000; setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, reinterpret_cast<char*>(&timeout), sizeof(timeout));
            std::string head;
            if (rewrite_head(client, host_port, broker_token, head)) {
                SOCKET upstream = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
                sockaddr_in target{}; target.sin_family = AF_INET; target.sin_port = htons(host_port); target.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
                if (upstream != INVALID_SOCKET && connect(upstream, reinterpret_cast<sockaddr*>(&target), sizeof(target)) == 0 && send_all(upstream, head.data(), head.size())) {
                    timeout = 0; setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, reinterpret_cast<char*>(&timeout), sizeof(timeout));
                    std::thread upload(relay_socket, client, upstream); relay_socket(upstream, client);
                    shutdown(client, SD_BOTH); shutdown(upstream, SD_BOTH); upload.join();
                }
                if (upstream != INVALID_SOCKET) closesocket(upstream);
            }
            closesocket(client); active->fetch_sub(1);
        }).detach();
    }
    closesocket(listener); WSACleanup(); return 127;
}
