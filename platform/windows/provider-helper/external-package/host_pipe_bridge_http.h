#pragma once
#include "contained_http.h"

namespace host_bridge {
struct RequestHead {
    std::string rewritten;
    size_t length = 0;
};
inline bool hex(const std::string& value, size_t length) {
    return value.size() == length && std::all_of(value.begin(), value.end(), [](unsigned char c) {
        return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
    });
}
inline bool request_head(const std::string& head, uint16_t port,
                         const std::string& broker_token, RequestHead& output) {
    contained_http::RequestHead parsed;
    if (!contained_http::parse(head, parsed)) return false;
    RequestHead rewritten; rewritten.length = parsed.length;
    rewritten.rewritten = parsed.request + "\r\n";
    for (const auto& field : parsed.fields) {
        if (field.first != "host" && field.first != "connection" && field.first != "proxy-connection"
            && (broker_token.empty() || (field.first != "authorization" && field.first != "x-api-key"))) {
            rewritten.rewritten += field.second + "\r\n";
        }
    }
    if (!broker_token.empty()) rewritten.rewritten += "Authorization: Bearer " + broker_token + "\r\n";
    rewritten.rewritten += "Host: 127.0.0.1:" + std::to_string(port) + "\r\nConnection: close\r\n\r\n";
    output = std::move(rewritten); return true;
}
}
