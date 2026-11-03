#pragma once
#include <algorithm>
#include <cstdint>
#include <set>
#include <string>
#include <utility>
#include <vector>

namespace contained_http {
struct RequestHead {
    std::string request;
    std::vector<std::pair<std::string, std::string>> fields;
    size_t length = 0;
};
inline bool parse(const std::string& head, RequestHead& output) {
    if (head.size() > 16384 || head.size() < 4 || head.compare(head.size() - 4, 4, "\r\n\r\n")) return false;
    const auto first = head.find("\r\n");
    const auto request = head.substr(0, first);
    const auto a = request.find(' '), b = request.find(' ', a + 1);
    if (a == std::string::npos || b == std::string::npos || request.substr(b + 1) != "HTTP/1.1") return false;
    const auto method = request.substr(0, a), path = request.substr(a + 1, b - a - 1);
    if ((method != "POST" && method != "GET") || path.empty() || path[0] != '/' || path.rfind("//", 0) == 0
        || path.find_first_of("#\\%") != std::string::npos
        || std::any_of(path.begin(), path.end(), [](unsigned char c) { return c <= 32 || c == 127; })) return false;
    RequestHead parsed; parsed.request = request;
    std::set<std::string> fields;
    bool length = false, json = false;
    for (size_t start = first + 2; start < head.size() - 2;) {
        const auto next = head.find("\r\n", start);
        if (next == std::string::npos) return false;
        const auto line = head.substr(start, next - start);
        const auto colon = line.find(':');
        if (colon == std::string::npos || !colon) return false;
        auto name = line.substr(0, colon), value = line.substr(colon + 1);
        if (!std::all_of(name.begin(), name.end(), [](unsigned char c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '-'; })
            || std::any_of(value.begin(), value.end(), [](unsigned char c) { return c < 32 || c == 127; })) return false;
        std::transform(name.begin(), name.end(), name.begin(), [](unsigned char c) { return static_cast<char>(c >= 'A' && c <= 'Z' ? c + ('a' - 'A') : c); });
        if (!fields.insert(name).second || name == "transfer-encoding" || name == "expect"
            || name == "upgrade" || name == "proxy-authorization") return false;
        while (!value.empty() && value.front() == ' ') value.erase(0, 1);
        while (!value.empty() && value.back() == ' ') value.pop_back();
        if (name == "content-length") {
            if (value.empty()) return false;
            for (const auto c : value) {
                if (c < '0' || c > '9' || parsed.length > 16777216 / 10) return false;
                parsed.length = parsed.length * 10 + static_cast<size_t>(c - '0');
                if (parsed.length > 16777216) return false;
            }
            length = true;
        }
        if (name == "content-type") json = value.substr(0, value.find(';')) == "application/json";
        parsed.fields.emplace_back(name, line);
        start = next + 2;
    }
    if (!fields.count("host") || (method == "POST" && (!length || !json)) || (method == "GET" && parsed.length)) return false;
    output = std::move(parsed); return true;
}
}
