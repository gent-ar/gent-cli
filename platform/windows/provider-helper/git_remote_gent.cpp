#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <cwchar>
#include <string>

static std::wstring environment_value(const wchar_t* name) {
    std::wstring value(32768, L'\0');
    const auto size = GetEnvironmentVariableW(name, value.data(), static_cast<DWORD>(value.size()));
    value.resize(size < value.size() ? size : 0);
    return value;
}

static void add_git_config(const std::wstring& key, const std::wstring& value) {
    const auto text = environment_value(L"GIT_CONFIG_COUNT");
    const auto count = text.empty() ? 0 : std::stoul(text);
    const auto index = std::to_wstring(count);
    SetEnvironmentVariableW((L"GIT_CONFIG_KEY_" + index).c_str(), key.c_str());
    SetEnvironmentVariableW((L"GIT_CONFIG_VALUE_" + index).c_str(), value.c_str());
    SetEnvironmentVariableW(L"GIT_CONFIG_COUNT", std::to_wstring(count + 1).c_str());
}

struct Endpoint {
    std::wstring host, port;
};

static Endpoint endpoint_of(const std::wstring& url) {
    const auto scheme_end = url.find(L"://");
    if (scheme_end == std::wstring::npos) return {};
    const auto start = scheme_end + 3;
    auto authority = url.substr(start, url.find_first_of(L"/?#", start) - start);
    const auto credentials = authority.rfind(L'@');
    if (credentials != std::wstring::npos) authority.erase(0, credentials + 1);
    if (authority.empty() || authority[0] == L'[') return {};
    const auto colon = authority.find(L':');
    const auto port = colon == std::wstring::npos ? (url.compare(0, scheme_end, L"https") == 0 ? L"443" : L"80") : authority.substr(colon + 1);
    return {authority.substr(0, colon), port};
}

static std::wstring resolved_addresses(const Endpoint& endpoint) {
    WSADATA data{};
    if (endpoint.host.empty() || WSAStartup(MAKEWORD(2, 2), &data)) return {};
    ADDRINFOW hints{};
    hints.ai_socktype = SOCK_STREAM;
    ADDRINFOW* found = nullptr;
    std::wstring addresses;
    if (!GetAddrInfoW(endpoint.host.c_str(), endpoint.port.c_str(), &hints, &found)) {
        for (auto* row = found; row; row = row->ai_next) {
            wchar_t text[64]{};
            const void* address = row->ai_family == AF_INET ? static_cast<const void*>(&reinterpret_cast<sockaddr_in*>(row->ai_addr)->sin_addr)
                                                            : static_cast<const void*>(&reinterpret_cast<sockaddr_in6*>(row->ai_addr)->sin6_addr);
            if (!InetNtopW(row->ai_family, address, text, 64)) continue;
            if (!addresses.empty()) addresses += L',';
            addresses += row->ai_family == AF_INET ? std::wstring(text) : L"[" + std::wstring(text) + L"]";
        }
        FreeAddrInfoW(found);
    }
    WSACleanup();
    return addresses;
}

static std::wstring quoted(const std::wstring& value) {
    std::wstring result = L"\"";
    size_t slashes = 0;
    for (const auto c : value) {
        if (c == L'\\') { ++slashes; continue; }
        result.append(c == L'"' ? slashes * 2 + 1 : slashes, L'\\');
        slashes = 0;
        result += c;
    }
    result.append(slashes * 2, L'\\');
    return result + L'"';
}

int wmain(int argc, wchar_t** argv) {
    if (argc != 3) return 2;
    const std::wstring url = argv[2];
    const auto endpoint = endpoint_of(url);
    const auto addresses = resolved_addresses(endpoint);
    if (!addresses.empty()) add_git_config(L"http.curloptResolve", endpoint.host + L":" + endpoint.port + L":" + addresses);
    add_git_config(L"http.sslBackend", L"schannel");
    auto line = L"git-remote-" + url.substr(0, url.find(L"://")) + L" " + quoted(argv[1]) + L" " + quoted(url);
    STARTUPINFOW startup{};
    startup.cb = sizeof(startup);
    PROCESS_INFORMATION created{};
    if (!CreateProcessW(nullptr, line.data(), nullptr, nullptr, TRUE, 0, nullptr, nullptr, &startup, &created)) return 127;
    CloseHandle(created.hThread);
    WaitForSingleObject(created.hProcess, INFINITE);
    DWORD code = 1;
    GetExitCodeProcess(created.hProcess, &code);
    CloseHandle(created.hProcess);
    return static_cast<int>(code);
}
